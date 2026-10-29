use std::{net::SocketAddr, time::Duration};

use anyhow::{bail, Context, Result};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    Development,
    Test,
    Production,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationMode {
    /// Any verified Google account may sign in.
    Open,
    /// Only people with a pending invitation (or an existing account) may sign in.
    InviteOnly,
    /// Verified emails in these domains may sign in; others need an invitation.
    Domains(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrgCreation {
    /// Any signed-in user may create an organization (and becomes its owner).
    Anyone,
    /// Only instance administrators may create organizations.
    InstanceAdmins,
}

#[derive(Clone, Debug)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub issuer_url: String,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub environment: Environment,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub database_max_connections: u32,
    pub public_url: Url,
    pub google: Option<GoogleConfig>,
    pub registration: RegistrationMode,
    pub org_creation: OrgCreation,
    pub session_ttl: Duration,
    pub encryption_key: Option<[u8; 32]>,
    pub dev_login: bool,
    pub discord_base_override: Option<Url>,
    pub metrics_token: Option<String>,
    pub request_timeout: Duration,
    pub trust_proxy_headers: bool,
    pub run_worker: bool,
    pub worker_poll_interval: Duration,
    pub log_json: bool,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn flag(name: &str, default: bool) -> Result<bool> {
    match var(name).as_deref() {
        None => Ok(default),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some("0" | "false" | "no" | "off") => Ok(false),
        Some(other) => bail!("{name} must be true or false, got {other:?}"),
    }
}

fn number<T: std::str::FromStr>(name: &str, default: T) -> Result<T> {
    match var(name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| anyhow::anyhow!("{name} must be a number, got {v:?}")),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let environment = match var("APP_ENV").as_deref().unwrap_or("development") {
            "development" | "dev" => Environment::Development,
            "test" => Environment::Test,
            "production" | "prod" => Environment::Production,
            other => bail!("APP_ENV must be development, test or production, got {other:?}"),
        };
        let production = environment == Environment::Production;

        let database_url = var("DATABASE_URL").context("DATABASE_URL is required")?;
        let public_url: Url = var("PUBLIC_URL").unwrap_or_else(|| "http://localhost:5173".into()).parse().context("PUBLIC_URL must be an absolute URL")?;
        if production && public_url.scheme() != "https" {
            bail!("PUBLIC_URL must use https in production");
        }

        let google = match (var("GOOGLE_CLIENT_ID"), var("GOOGLE_CLIENT_SECRET")) {
            (Some(client_id), Some(client_secret)) => {
                Some(GoogleConfig { client_id, client_secret, issuer_url: var("GOOGLE_ISSUER_URL").unwrap_or_else(|| "https://accounts.google.com".into()) })
            }
            (None, None) => None,
            _ => bail!("set both GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET, or neither"),
        };

        let registration = match var("REGISTRATION_MODE").as_deref().unwrap_or(if production { "invite_only" } else { "open" }) {
            "open" => RegistrationMode::Open,
            "invite_only" => RegistrationMode::InviteOnly,
            "domains" => {
                let domains: Vec<String> = var("ALLOWED_EMAIL_DOMAINS")
                    .context("REGISTRATION_MODE=domains requires ALLOWED_EMAIL_DOMAINS")?
                    .split(',')
                    .map(|d| d.trim().trim_start_matches('@').to_ascii_lowercase())
                    .filter(|d| !d.is_empty())
                    .collect();
                if domains.is_empty() {
                    bail!("ALLOWED_EMAIL_DOMAINS is empty");
                }
                RegistrationMode::Domains(domains)
            }
            other => bail!("REGISTRATION_MODE must be open, invite_only or domains, got {other:?}"),
        };

        let org_creation = match var("ORG_CREATION").as_deref().unwrap_or(if production { "instance_admins" } else { "anyone" }) {
            "anyone" => OrgCreation::Anyone,
            "instance_admins" => OrgCreation::InstanceAdmins,
            other => bail!("ORG_CREATION must be anyone or instance_admins, got {other:?}"),
        };

        let encryption_key = match var("ENCRYPTION_KEY") {
            Some(encoded) => Some(crate::crypto::parse_key(&encoded)?),
            None if production => bail!("ENCRYPTION_KEY is required in production (generate one with `openssl rand -base64 32`)"),
            None => None,
        };

        let dev_login = flag("AUTH_DEV_LOGIN", false)?;
        if dev_login && production {
            bail!("AUTH_DEV_LOGIN cannot be enabled in production");
        }
        if google.is_none() && !dev_login {
            bail!("no sign-in method configured: set GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET (or AUTH_DEV_LOGIN=true outside production)");
        }

        let discord_base_override = match var("DISCORD_API_BASE_OVERRIDE") {
            Some(_) if production => bail!("DISCORD_API_BASE_OVERRIDE is for tests and local development only"),
            Some(v) => Some(v.parse().context("DISCORD_API_BASE_OVERRIDE must be a URL")?),
            None => None,
        };

        Ok(Config {
            environment,
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()).parse().context("BIND_ADDR must be host:port")?,
            database_url,
            database_max_connections: number("DATABASE_MAX_CONNECTIONS", 10)?,
            public_url,
            google,
            registration,
            org_creation,
            session_ttl: Duration::from_secs(number("SESSION_TTL_HOURS", 24 * 7u64)? * 3600),
            encryption_key,
            dev_login,
            discord_base_override,
            metrics_token: var("METRICS_TOKEN"),
            request_timeout: Duration::from_secs(number("REQUEST_TIMEOUT_SECONDS", 30u64)?),
            trust_proxy_headers: flag("TRUST_PROXY_HEADERS", false)?,
            run_worker: flag("RUN_WORKER", true)?,
            worker_poll_interval: Duration::from_millis(number("WORKER_POLL_INTERVAL_MS", 2000u64)?),
            log_json: flag("LOG_JSON", production)?,
        })
    }

    pub fn is_production(&self) -> bool {
        self.environment == Environment::Production
    }

    /// Cookies carry the Secure flag whenever the public URL is https.
    pub fn secure_cookies(&self) -> bool {
        self.public_url.scheme() == "https"
    }

    pub fn session_cookie_name(&self) -> &'static str {
        if self.secure_cookies() {
            "__Host-oncallkit_session"
        } else {
            "oncallkit_session"
        }
    }

    pub fn app_url(&self, path: &str) -> String {
        let base = self.public_url.as_str().trim_end_matches('/');
        format!("{base}{path}")
    }

    /// Minimal configuration for tests.
    pub fn for_tests(database_url: &str) -> Self {
        Config {
            environment: Environment::Test,
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            database_url: database_url.into(),
            database_max_connections: 5,
            public_url: "http://localhost:5173".parse().unwrap(),
            google: None,
            registration: RegistrationMode::Open,
            org_creation: OrgCreation::Anyone,
            session_ttl: Duration::from_secs(3600),
            encryption_key: Some([7u8; 32]),
            dev_login: true,
            discord_base_override: None,
            metrics_token: None,
            request_timeout: Duration::from_secs(10),
            trust_proxy_headers: false,
            run_worker: false,
            worker_poll_interval: Duration::from_millis(100),
            log_json: false,
        }
    }
}
