//! Discord webhook delivery and URL validation.
//!
//! Only real Discord webhook URLs are accepted, which closes off server-side request forgery:
//! the host must be one of Discord's own, the scheme https, no credentials, no custom port, and
//! the path must be a webhook path. Redirects are never followed.

use std::time::Duration;

use reqwest::StatusCode;
use serde::Serialize;
use url::Url;

const DISCORD_HOSTS: [&str; 4] = ["discord.com", "discordapp.com", "ptb.discord.com", "canary.discord.com"];

#[derive(Debug, PartialEq, Eq)]
pub struct WebhookUrl {
    pub url: Url,
    pub webhook_id: String,
}

pub fn validate_webhook_url(raw: &str) -> Result<WebhookUrl, &'static str> {
    let url = Url::parse(raw.trim()).map_err(|_| "enter the full webhook URL from Discord")?;
    if url.scheme() != "https" {
        return Err("the webhook URL must start with https://");
    }
    let host = url.host_str().ok_or("the webhook URL has no host")?.to_ascii_lowercase();
    if !DISCORD_HOSTS.contains(&host.as_str()) {
        return Err("only discord.com webhook URLs are accepted");
    }
    if url.port().is_some_and(|p| p != 443) || !url.username().is_empty() || url.password().is_some() {
        return Err("the webhook URL must not contain a port or credentials");
    }
    if url.fragment().is_some() {
        return Err("the webhook URL must not contain a fragment");
    }
    if let Some(query) = url.query() {
        // Discord allows targeting a forum thread; nothing else belongs in the query.
        if !query.split('&').all(|pair| pair.starts_with("thread_id=") && pair[10..].chars().all(|c| c.is_ascii_digit())) {
            return Err("the webhook URL has unexpected query parameters");
        }
    }
    let segments: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let (id, token) = match segments.as_slice() {
        ["api", "webhooks", id, token] => (id.to_string(), token.to_string()),
        ["api", version, "webhooks", id, token] if version.starts_with('v') && version[1..].chars().all(|c| c.is_ascii_digit()) => {
            (id.to_string(), token.to_string())
        }
        _ => return Err("this does not look like a Discord webhook URL (…/api/webhooks/<id>/<token>)"),
    };
    if id.is_empty() || id.len() > 25 || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("the webhook id in the URL is not valid");
    }
    if token.len() < 20 || token.len() > 200 || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("the webhook token in the URL is not valid");
    }
    Ok(WebhookUrl { url, webhook_id: id })
}

/// A display hint that identifies a webhook without revealing its token.
pub fn mask(webhook: &WebhookUrl) -> String {
    format!("{}/…/webhooks/{}/••••••", webhook.url.host_str().unwrap_or("discord.com"), webhook.webhook_id)
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscordMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub username: String,
    pub embeds: Vec<Embed>,
    pub allowed_mentions: AllowedMentions,
}

#[derive(Debug, Clone, Serialize)]
pub struct Embed {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub color: u32,
    pub fields: Vec<EmbedField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub footer: Option<EmbedFooter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbedFooter {
    pub text: String,
}

/// Mentions are opt-in and limited to the listed user ids, so names or templates can never
/// trigger @everyone or role pings.
#[derive(Debug, Clone, Serialize)]
pub struct AllowedMentions {
    pub parse: Vec<String>,
    pub users: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Delivered { status: u16 },
    Retry { status: Option<u16>, error: String, retry_after: Option<Duration> },
    Failed { status: Option<u16>, error: String },
}

#[derive(Clone)]
pub struct DiscordClient {
    http: reqwest::Client,
    base_override: Option<Url>,
}

impl DiscordClient {
    pub fn new(base_override: Option<Url>) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .user_agent(concat!("OncallKit/", env!("CARGO_PKG_VERSION"), " (+https://github.com/rikyrism/OncallKit)"))
            .build()
            .expect("HTTP client");
        DiscordClient { http, base_override }
    }

    fn target(&self, webhook: &WebhookUrl) -> Url {
        let mut url = webhook.url.clone();
        if let Some(base) = &self.base_override {
            // Development/test only (refused in production): send to a mock server instead.
            let _ = url.set_scheme(base.scheme());
            let _ = url.set_host(base.host_str());
            let _ = url.set_port(base.port());
        }
        url.query_pairs_mut().append_pair("wait", "true");
        url
    }

    pub async fn send(&self, webhook_url: &str, message: &DiscordMessage) -> Outcome {
        let webhook = match validate_webhook_url(webhook_url) {
            Ok(w) => w,
            Err(reason) => return Outcome::Failed { status: None, error: format!("invalid webhook URL: {reason}") },
        };
        let response = match self.http.post(self.target(&webhook)).json(message).send().await {
            Ok(r) => r,
            // Never include the URL in the error: it carries the webhook token.
            Err(err) => {
                let kind = if err.is_timeout() {
                    "timed out"
                } else if err.is_connect() {
                    "could not connect"
                } else {
                    "request failed"
                };
                return Outcome::Retry { status: None, error: format!("Discord {kind}"), retry_after: None };
            }
        };
        let status = response.status();
        let retry_after = retry_after(response.headers());
        if status.is_success() {
            return Outcome::Delivered { status: status.as_u16() };
        }
        let body: String = response.text().await.unwrap_or_default().chars().take(300).collect();
        let error = format!("Discord answered {}{}", status.as_u16(), if body.is_empty() { String::new() } else { format!(": {body}") });
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
            Outcome::Retry { status: Some(status.as_u16()), error, retry_after }
        } else {
            Outcome::Failed { status: Some(status.as_u16()), error }
        }
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get("retry-after").or_else(|| headers.get("x-ratelimit-reset-after"))?.to_str().ok()?;
    let seconds: f64 = value.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds.min(3600.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyz_ABCDEFG-0123456789";

    #[test]
    fn accepts_discord_webhooks() {
        for url in [
            format!("https://discord.com/api/webhooks/123456789012345678/{TOKEN}"),
            format!("https://discordapp.com/api/webhooks/123456789012345678/{TOKEN}"),
            format!("https://discord.com/api/v10/webhooks/123456789012345678/{TOKEN}"),
            format!("https://canary.discord.com/api/webhooks/123456789012345678/{TOKEN}?thread_id=42"),
        ] {
            assert!(validate_webhook_url(&url).is_ok(), "{url}");
        }
    }

    #[test]
    fn rejects_ssrf_and_malformed_urls() {
        for url in [
            format!("http://discord.com/api/webhooks/1/{TOKEN}"),
            format!("https://discord.com.evil.io/api/webhooks/1/{TOKEN}"),
            format!("https://evil.io/api/webhooks/1/{TOKEN}"),
            format!("https://169.254.169.254/api/webhooks/1/{TOKEN}"),
            format!("https://localhost/api/webhooks/1/{TOKEN}"),
            format!("https://discord.com:8443/api/webhooks/1/{TOKEN}"),
            format!("https://user:pass@discord.com/api/webhooks/1/{TOKEN}"),
            format!("https://discord.com/api/webhooks/abc/{TOKEN}"),
            format!("https://discord.com/api/webhooks/1/{TOKEN}/extra"),
            format!("https://discord.com/api/webhooks/1/{TOKEN}?redirect=http://x"),
            "https://discord.com/api/webhooks/1/short".to_string(),
            "not a url".to_string(),
        ] {
            assert!(validate_webhook_url(&url).is_err(), "{url}");
        }
    }

    #[test]
    fn mask_hides_the_token() {
        let w = validate_webhook_url(&format!("https://discord.com/api/webhooks/123/{TOKEN}")).unwrap();
        let masked = mask(&w);
        assert!(!masked.contains(TOKEN));
        assert!(masked.contains("123"));
    }

    #[test]
    fn parses_retry_after() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("retry-after", "1.5".parse().unwrap());
        assert_eq!(retry_after(&h), Some(Duration::from_millis(1500)));
    }
}
