//! iCalendar (RFC 5545) subscription feeds, so on-call shifts show up in Google Calendar,
//! Apple Calendar or Outlook.
//!
//! Calendar apps fetch the feed without cookies, so each feed carries a random secret token in
//! its URL; only the token's hash is stored and the link is shown once. The token travels in the
//! query string because request logs record only the path. A feed stops working as soon as it is
//! revoked, its owner leaves the organization or is disabled, or its schedule is deleted.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::{ClientIp, OrgContext},
    crypto::{hash_token, random_token},
    error::{AppError, AppResult},
    schedules::{
        engine::{Shift, ShiftSource},
        repo,
    },
    state::AppState,
};

/// How far back and ahead a feed reaches. Calendar apps re-fetch periodically, so the window
/// moves along with time.
const PAST: Duration = Duration::days(30);
const AHEAD: Duration = Duration::days(180);
/// Feeds one person may hold in one organization.
const MAX_FEEDS: i64 = 20;

#[derive(Serialize, sqlx::FromRow)]
pub struct Feed {
    id: Uuid,
    schedule_id: Option<Uuid>,
    schedule_name: Option<String>,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

const FEED_SELECT: &str = "SELECT f.id, f.schedule_id, s.name AS schedule_name, f.created_at, f.last_used_at
     FROM calendar_feeds f LEFT JOIN schedules s ON s.id = f.schedule_id";

pub async fn list(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Vec<Feed>>> {
    let feeds = sqlx::query_as(&format!("{FEED_SELECT} WHERE f.user_id = $1 AND f.organization_id = $2 ORDER BY f.created_at"))
        .bind(ctx.user_id())
        .bind(ctx.organization_id)
        .fetch_all(&state.db)
        .await?;
    Ok(Json(feeds))
}

#[derive(Deserialize)]
pub struct CreateFeed {
    /// Omit or null for "my shifts" across the organization.
    schedule_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct CreatedFeed {
    #[serde(flatten)]
    feed: Feed,
    /// The only time the link is shown. Anyone who has it can read the calendar.
    url: String,
    /// The same link with the webcal:// scheme, which opens the subscribe dialog directly.
    webcal_url: String,
}

pub async fn create(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<CreateFeed>) -> AppResult<(StatusCode, Json<CreatedFeed>)> {
    let mut tx = state.db.begin().await?;
    if let Some(schedule_id) = body.schedule_id {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM schedules WHERE id = $1 AND organization_id = $2)")
            .bind(schedule_id)
            .bind(ctx.organization_id)
            .fetch_one(&mut *tx)
            .await?;
        if !exists {
            return Err(AppError::validation("schedule_id", "schedule not found"));
        }
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM calendar_feeds WHERE user_id = $1 AND organization_id = $2")
        .bind(ctx.user_id())
        .bind(ctx.organization_id)
        .fetch_one(&mut *tx)
        .await?;
    if count >= MAX_FEEDS {
        return Err(AppError::Conflict(format!("you can have at most {MAX_FEEDS} calendar links; revoke one you no longer use")));
    }
    let token = random_token();
    let id: Uuid = sqlx::query_scalar("INSERT INTO calendar_feeds (user_id, organization_id, schedule_id, token_hash) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(ctx.user_id())
        .bind(ctx.organization_id)
        .bind(body.schedule_id)
        .bind(hash_token(&token))
        .fetch_one(&mut *tx)
        .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "calendar_feed.created",
        "calendar_feed",
        Some(id),
        json!({ "schedule_id": body.schedule_id }),
    )
    .await?;
    let feed: Feed = sqlx::query_as(&format!("{FEED_SELECT} WHERE f.id = $1")).bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    let url = state.config.app_url(&format!("/api/v1/calendar.ics?token={token}"));
    let webcal_url = url.replacen("https://", "webcal://", 1).replacen("http://", "webcal://", 1);
    Ok((StatusCode::CREATED, Json(CreatedFeed { feed, url, webcal_url })))
}

pub async fn revoke(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("DELETE FROM calendar_feeds WHERE id = $1 AND user_id = $2 AND organization_id = $3")
        .bind(id)
        .bind(ctx.user_id())
        .bind(ctx.organization_id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "calendar_feed.revoked", "calendar_feed", Some(id), json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct FeedQuery {
    token: Option<String>,
}

/// The feed itself. Public, authenticated only by the secret token.
pub async fn feed(State(state): State<AppState>, ClientIp(ip): ClientIp, Query(q): Query<FeedQuery>) -> AppResult<Response> {
    if !state.write_limiter.check(ip, "calendar") {
        return Err(AppError::RateLimited);
    }
    let token = q.token.filter(|t| !t.is_empty() && t.len() <= 128).ok_or(AppError::NotFound)?;
    // The owner must still be an active member of the organization.
    let row: Option<(Uuid, Uuid, Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT f.id, f.user_id, f.organization_id, f.schedule_id, o.name FROM calendar_feeds f
         JOIN users u ON u.id = f.user_id AND u.disabled_at IS NULL
         JOIN organization_memberships m ON m.organization_id = f.organization_id AND m.user_id = f.user_id
         JOIN organizations o ON o.id = f.organization_id
         WHERE f.token_hash = $1",
    )
    .bind(hash_token(&token))
    .fetch_optional(&state.db)
    .await?;
    let (feed_id, user_id, organization_id, schedule_id, org_name) = row.ok_or(AppError::NotFound)?;

    let now = Utc::now();
    let (from, to) = (now - PAST, now + AHEAD);
    let (calendar_name, events) = match schedule_id {
        Some(schedule_id) => {
            let loaded = repo::load_one(&state.db, organization_id, schedule_id).await?;
            let shifts = if loaded.row.status == "archived" { vec![] } else { loaded.spec.shifts(from, to) };
            let names = people(&state, &shifts).await?;
            let events = shifts
                .iter()
                .map(|s| {
                    let who = names.get(&s.user_id).cloned().unwrap_or_else(|| "Someone".into());
                    event(&state, &loaded.row.id, &loaded.row.name, s, format!("{who} on call: {} ({})", loaded.row.name, s.layer.as_str()))
                })
                .collect();
            (format!("{} on call", loaded.row.name), events)
        }
        None => {
            let schedules = repo::load_many(&state.db, organization_id, "AND status <> 'archived'").await?;
            let mut events = Vec::new();
            for l in &schedules {
                for s in l.spec.shifts(from, to).iter().filter(|s| s.user_id == user_id) {
                    events.push(event(&state, &l.row.id, &l.row.name, s, format!("On call: {} ({})", l.row.name, s.layer.as_str())));
                }
            }
            (format!("My on-call ({org_name})"), events)
        }
    };

    // Calendar apps poll; recording every fetch would only churn the row.
    sqlx::query("UPDATE calendar_feeds SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '10 minutes')")
        .bind(feed_id)
        .execute(&state.db)
        .await?;

    let body = render(&calendar_name, &events, now);
    Ok((
        [
            (header::CONTENT_TYPE, "text/calendar; charset=utf-8"),
            (header::CACHE_CONTROL, "private, max-age=300"),
            (header::CONTENT_DISPOSITION, "inline; filename=\"oncallkit.ics\""),
        ],
        body,
    )
        .into_response())
}

async fn people(state: &AppState, shifts: &[Shift]) -> sqlx::Result<std::collections::HashMap<Uuid, String>> {
    let mut ids: Vec<Uuid> = shifts.iter().map(|s| s.user_id).collect();
    ids.sort();
    ids.dedup();
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, display_name FROM users WHERE id = ANY($1)").bind(&ids).fetch_all(&state.db).await?;
    Ok(rows.into_iter().collect())
}

fn event(state: &AppState, schedule_id: &Uuid, schedule_name: &str, shift: &Shift, summary: String) -> CalendarEvent {
    let url = state.config.app_url(&format!("/schedules/{schedule_id}"));
    let mut description = format!("Schedule: {schedule_name}\nLayer: {}", shift.layer.as_str());
    if matches!(shift.source, ShiftSource::Override { .. }) {
        description.push_str("\nThis shift is an override.");
    }
    description.push_str(&format!("\n{url}"));
    CalendarEvent {
        // Stable across fetches, so an edited schedule updates events instead of duplicating them.
        uid: format!("{schedule_id}-{}-{}@oncallkit", shift.layer.as_str(), shift.start.timestamp()),
        start: shift.start,
        end: shift.end,
        summary,
        description,
        url,
    }
}

pub struct CalendarEvent {
    pub uid: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub summary: String,
    pub description: String,
    pub url: String,
}

/// Render a VCALENDAR. Times are written in UTC, which every client converts to its own zone.
pub fn render(name: &str, events: &[CalendarEvent], now: DateTime<Utc>) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".into(),
        "PRODID:-//OncallKit//OncallKit//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        format!("X-WR-CALNAME:{}", escape(name)),
        "REFRESH-INTERVAL;VALUE=DURATION:PT1H".into(),
        "X-PUBLISHED-TTL:PT1H".into(),
    ];
    for e in events {
        lines.extend([
            "BEGIN:VEVENT".to_string(),
            format!("UID:{}", escape(&e.uid)),
            format!("DTSTAMP:{}", stamp(now)),
            format!("DTSTART:{}", stamp(e.start)),
            format!("DTEND:{}", stamp(e.end)),
            format!("SUMMARY:{}", escape(&e.summary)),
            format!("DESCRIPTION:{}", escape(&e.description)),
            format!("URL:{}", e.url),
            "TRANSP:OPAQUE".into(),
            "END:VEVENT".into(),
        ]);
    }
    lines.push("END:VCALENDAR".into());
    let mut out = String::new();
    for line in lines {
        out.push_str(&fold(&line));
        out.push_str("\r\n");
    }
    out
}

fn stamp(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Escape a TEXT value (RFC 5545 §3.3.11).
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Fold a content line to at most 75 octets per physical line, never splitting a UTF-8
/// character (RFC 5545 §3.1).
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3);
    let mut used = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if used + len > 75 {
            out.push_str("\r\n ");
            used = 1;
        }
        out.push(c);
        used += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_text_values() {
        assert_eq!(escape("a;b,c\\d\ne\r"), "a\\;b\\,c\\\\d\\ne");
    }

    #[test]
    fn folds_long_lines_without_splitting_characters() {
        let line = format!("SUMMARY:{}", "é".repeat(60));
        let folded = fold(&line);
        for physical in folded.split("\r\n") {
            assert!(physical.len() <= 75, "{} octets", physical.len());
        }
        assert_eq!(folded.replace("\r\n ", ""), line);
        assert_eq!(fold("SHORT:line"), "SHORT:line");
    }

    #[test]
    fn renders_a_calendar() {
        let start: DateTime<Utc> = "2026-11-03T02:00:00Z".parse().unwrap();
        let event = CalendarEvent {
            uid: "s-primary-1@oncallkit".into(),
            start,
            end: start + Duration::days(7),
            summary: "On call: DevOps, SRE (primary)".into(),
            description: "Schedule: DevOps\nLayer: primary".into(),
            url: "https://oncall.example.com/schedules/s".into(),
        };
        let ics = render("My on-call", &[event], start);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(ics.contains("DTSTART:20261103T020000Z\r\n"));
        assert!(ics.contains("DTEND:20261110T020000Z\r\n"));
        assert!(ics.contains("SUMMARY:On call: DevOps\\, SRE (primary)\r\n"));
        assert!(ics.contains("DESCRIPTION:Schedule: DevOps\\nLayer: primary\r\n"));
        assert!(!ics.replace("\r\n", "").contains('\n'), "only CRLF line endings");
    }
}
