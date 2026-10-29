//! Rendering events as Discord messages. Times are shown in the schedule's timezone.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use super::{EventKind, EventPayload, PersonRef};
use crate::integrations::discord::{AllowedMentions, DiscordMessage, Embed, EmbedField, EmbedFooter};

const INK: u32 = 0x1A1A17;
const SIGNAL: u32 = 0xF2C200;
const ALERT: u32 = 0xC8341E;

pub struct RenderOptions<'a> {
    pub app_url: &'a str,
    pub mention_users: bool,
    pub prefix: &'a str,
}

pub fn format_time(t: DateTime<Utc>, timezone: Option<&str>) -> String {
    let tz: Tz = timezone.and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::UTC);
    t.with_timezone(&tz).format("%-d %B %Y, %H:%M %Z").to_string()
}

fn who(person: &Option<PersonRef>, mention: bool, mentions: &mut Vec<String>) -> String {
    match person {
        Some(p) => match (&p.discord_user_id, mention) {
            (Some(id), true) => {
                mentions.push(id.clone());
                format!("{} (<@{id}>)", escape(&p.name))
            }
            _ => escape(&p.name),
        },
        None => "Nobody".into(),
    }
}

/// Neutralise Discord markdown and mention syntax in user-provided text.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().take(200) {
        if matches!(c, '*' | '_' | '`' | '~' | '|' | '>' | '#' | '[' | ']' | '(' | ')' | '\\') {
            out.push('\\');
        }
        if c == '@' {
            out.push_str("@\u{200B}");
            continue;
        }
        out.push(c);
    }
    out
}

pub fn render(kind: EventKind, payload: &EventPayload, opts: &RenderOptions) -> DiscordMessage {
    let tz = payload.timezone.as_deref();
    let schedule = payload.schedule_name.as_deref().map(escape).unwrap_or_else(|| "Schedule".into());
    let mut mentions = Vec::new();
    let mut fields = Vec::new();
    let mut field = |name: &str, value: String, inline: bool| fields.push(EmbedField { name: name.into(), value, inline });
    let (title, description, color) = match kind {
        EventKind::RotationChanged | EventKind::PrimaryChanged | EventKind::SecondaryChanged => {
            field("Primary", who(&payload.primary, opts.mention_users, &mut mentions), true);
            if payload.secondary.is_some() || payload.previous_secondary.is_some() {
                field("Secondary", who(&payload.secondary, opts.mention_users, &mut mentions), true);
            }
            if payload.previous_primary.is_some() {
                field("Previous primary", who(&payload.previous_primary, false, &mut mentions), true);
            }
            if let Some(at) = payload.at {
                field("Effective from", format_time(at, tz), false);
            }
            if let Some(next) = payload.next_rotation_at {
                field("Next rotation", format_time(next, tz), false);
            }
            let cause = if payload.cause.as_deref() == Some("override") { "Override in effect" } else { "On-call handover" };
            (format!("{schedule}: {cause}"), None, SIGNAL)
        }
        EventKind::RotationApproaching => {
            let (layer, person) = if payload.layer.as_deref() == Some("secondary") { ("Secondary", &payload.secondary) } else { ("Primary", &payload.primary) };
            field(layer, who(person, opts.mention_users, &mut mentions), true);
            if let Some(at) = payload.at {
                field("Shift starts", format_time(at, tz), false);
            }
            if let Some(end) = payload.shift_end {
                field("Shift ends", format_time(end, tz), false);
            }
            let lead = payload.offset_minutes.map(humanize_minutes).unwrap_or_default();
            (format!("{schedule}: shift starts in {lead}"), None, INK)
        }
        EventKind::ScheduleCreated => {
            if let Some(at) = payload.at {
                field("Starts", format_time(at, tz), false);
            }
            (format!("{schedule}: schedule created"), payload.actor_name.as_deref().map(|a| format!("Created by {}", escape(a))), INK)
        }
        EventKind::ScheduleUpdated => (
            format!("{schedule}: schedule updated"),
            Some(format!(
                "{}{}",
                payload.detail.as_deref().unwrap_or(""),
                payload.actor_name.as_deref().map(|a| format!(" Changed by {}.", escape(a))).unwrap_or_default()
            )),
            INK,
        ),
        EventKind::OverrideCreated | EventKind::OverrideCanceled => {
            let layer = if payload.layer.as_deref() == Some("secondary") { "Secondary" } else { "Primary" };
            let person = if layer == "Secondary" { &payload.secondary } else { &payload.primary };
            field(&format!("{layer} cover"), who(person, kind == EventKind::OverrideCreated && opts.mention_users, &mut mentions), true);
            let replaced = if layer == "Secondary" { &payload.previous_secondary } else { &payload.previous_primary };
            if replaced.is_some() {
                field("Instead of", who(replaced, false, &mut mentions), true);
            }
            if let (Some(a), Some(b)) = (payload.at, payload.shift_end) {
                field("From", format_time(a, tz), false);
                field("Until", format_time(b, tz), false);
            }
            if let Some(reason) = &payload.reason {
                field("Reason", escape(reason), false);
            }
            let verb = if kind == EventKind::OverrideCreated { "override added" } else { "override canceled" };
            (format!("{schedule}: {verb}"), None, if kind == EventKind::OverrideCreated { SIGNAL } else { INK })
        }
        EventKind::DeliveryFailed => (
            "OncallKit could not deliver a notification".into(),
            Some(payload.detail.as_deref().map(escape).unwrap_or_else(|| "A notification failed after all retries.".into())),
            ALERT,
        ),
        EventKind::Test => {
            ("OncallKit test message".into(), Some("This integration is connected. Notifications for your schedules will appear here.".into()), INK)
        }
    };
    if let Some(team) = &payload.team_name {
        field("Team", escape(team), true);
    }
    if payload.schedule_name.is_some() {
        field("Schedule", schedule.clone(), true);
    }
    let url = payload.schedule_id.map(|id| format!("{}/schedules/{id}", opts.app_url.trim_end_matches('/')));
    let prefix = opts.prefix.trim();
    let mut content = if prefix.is_empty() { String::new() } else { escape(prefix) };
    if !mentions.is_empty() {
        let tags: Vec<String> = mentions.iter().map(|m| format!("<@{m}>")).collect();
        content = format!("{} {}", content, tags.join(" ")).trim().to_string();
    }
    DiscordMessage {
        content: (!content.is_empty()).then_some(content),
        username: "OncallKit".into(),
        embeds: vec![Embed { title, description, url, color, fields, footer: Some(EmbedFooter { text: "OncallKit".into() }) }],
        allowed_mentions: AllowedMentions { parse: vec![], users: mentions },
    }
}

pub fn humanize_minutes(minutes: i32) -> String {
    match minutes {
        m if m % 1440 == 0 => plural(m / 1440, "day"),
        m if m % 60 == 0 => plural(m / 60, "hour"),
        m => plural(m, "minute"),
    }
}

fn plural(n: i32, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn person(name: &str, discord: Option<&str>) -> Option<PersonRef> {
        Some(PersonRef { user_id: Uuid::new_v4(), name: name.into(), discord_user_id: discord.map(String::from) })
    }

    #[test]
    fn handover_message_shows_people_and_local_times() {
        let payload = EventPayload {
            schedule_id: Some(Uuid::nil()),
            schedule_name: Some("DevOps On-call".into()),
            team_name: Some("DevOps".into()),
            timezone: Some("Asia/Jakarta".into()),
            at: Some("2026-11-03T02:00:00Z".parse().unwrap()),
            next_rotation_at: Some("2026-11-10T02:00:00Z".parse().unwrap()),
            primary: person("Andi", Some("123456789")),
            secondary: person("Budi", None),
            previous_primary: person("Riky", None),
            ..Default::default()
        };
        let msg = render(EventKind::RotationChanged, &payload, &RenderOptions { app_url: "https://oncall.example", mention_users: true, prefix: "" });
        let embed = &msg.embeds[0];
        let get = |n: &str| embed.fields.iter().find(|f| f.name == n).map(|f| f.value.clone()).unwrap();
        assert_eq!(get("Effective from"), "3 November 2026, 09:00 WIB");
        assert_eq!(get("Next rotation"), "10 November 2026, 09:00 WIB");
        assert!(get("Primary").starts_with("Andi"));
        assert_eq!(get("Previous primary"), "Riky");
        assert_eq!(embed.url.as_deref(), Some("https://oncall.example/schedules/00000000-0000-0000-0000-000000000000"));
        assert_eq!(msg.allowed_mentions.users, vec!["123456789".to_string()]);
        assert!(msg.allowed_mentions.parse.is_empty());
    }

    #[test]
    fn names_cannot_inject_mentions_or_markdown() {
        let escaped = escape("@everyone **bold**");
        assert!(!escaped.contains("@everyone"));
        assert!(escaped.contains("\\*\\*"));
    }

    #[test]
    fn humanizes_offsets() {
        assert_eq!(humanize_minutes(1440), "1 day");
        assert_eq!(humanize_minutes(120), "2 hours");
        assert_eq!(humanize_minutes(15), "15 minutes");
    }
}
