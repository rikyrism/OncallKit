pub mod discord;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use self::discord::{mask, validate_webhook_url, Outcome};
use crate::{
    audit,
    auth::OrgContext,
    error::{AppError, AppResult},
    notifications::{message, EventKind, EventPayload},
    rbac::Permission,
    state::AppState,
    teams::ensure_team,
};

/// Integrations as returned by the API. The webhook URL is write-only: after saving, only a
/// masked hint is ever returned.
#[derive(Serialize, sqlx::FromRow)]
pub struct Integration {
    id: Uuid,
    kind: String,
    name: String,
    team_id: Option<Uuid>,
    enabled: bool,
    events: Vec<String>,
    mention_users: bool,
    message_prefix: String,
    max_attempts: i32,
    webhook_hint: String,
    encrypted: bool,
    last_delivery_status: Option<String>,
    last_delivery_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

const SELECT: &str = "SELECT id, kind, name, team_id, enabled, events, mention_users, message_prefix, max_attempts,
        secret_hint AS webhook_hint, secret_nonce IS NOT NULL AS encrypted, last_delivery_status, last_delivery_at, created_at
     FROM integrations";

#[derive(Serialize)]
pub struct EventOption {
    key: &'static str,
}

pub async fn event_options() -> Json<Vec<&'static str>> {
    Json(EventKind::SUBSCRIBABLE.iter().map(|k| k.as_str()).collect())
}

pub async fn list(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Vec<Integration>>> {
    ctx.require(Permission::ViewIntegrations)?;
    let items = sqlx::query_as(&format!("{SELECT} WHERE organization_id = $1 ORDER BY lower(name)")).bind(ctx.organization_id).fetch_all(&state.db).await?;
    Ok(Json(items))
}

async fn fetch(state: &AppState, org: Uuid, id: Uuid) -> AppResult<Integration> {
    sqlx::query_as(&format!("{SELECT} WHERE organization_id = $1 AND id = $2")).bind(org).bind(id).fetch_optional(&state.db).await?.ok_or(AppError::NotFound)
}

#[derive(Deserialize)]
pub struct DiscordInput {
    name: Option<String>,
    webhook_url: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    team_id: Option<Option<Uuid>>,
    enabled: Option<bool>,
    events: Option<Vec<String>>,
    mention_users: Option<bool>,
    message_prefix: Option<String>,
    max_attempts: Option<i32>,
}

fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

fn validate_events(events: &[String]) -> AppResult<Vec<String>> {
    let mut out = Vec::new();
    for e in events {
        let kind = EventKind::parse(e).filter(|k| *k != EventKind::Test).ok_or_else(|| AppError::validation("events", format!("unknown event {e:?}")))?;
        if !out.contains(&kind.as_str().to_string()) {
            out.push(kind.as_str().to_string());
        }
    }
    if out.is_empty() {
        return Err(AppError::validation("events", "choose at least one event"));
    }
    Ok(out)
}

fn validate_common(body: &DiscordInput) -> AppResult<()> {
    if let Some(name) = &body.name {
        if name.trim().is_empty() || name.trim().chars().count() > 120 {
            return Err(AppError::validation("name", "use 1 to 120 characters"));
        }
    }
    if body.message_prefix.as_ref().is_some_and(|p| p.chars().count() > 200) {
        return Err(AppError::validation("message_prefix", "keep it under 200 characters"));
    }
    if body.max_attempts.is_some_and(|m| !(1..=20).contains(&m)) {
        return Err(AppError::validation("max_attempts", "choose between 1 and 20"));
    }
    Ok(())
}

pub async fn create_discord(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<DiscordInput>) -> AppResult<(StatusCode, Json<Integration>)> {
    ctx.require(Permission::ManageIntegrations)?;
    validate_common(&body)?;
    let name = body.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| AppError::validation("name", "name the integration"))?;
    let raw = body.webhook_url.as_deref().ok_or_else(|| AppError::validation("webhook_url", "paste the webhook URL from Discord"))?;
    let webhook = validate_webhook_url(raw).map_err(|e| AppError::validation("webhook_url", e))?;
    let events = validate_events(body.events.as_deref().unwrap_or(&EventKind::SUBSCRIBABLE.iter().map(|k| k.as_str().to_string()).collect::<Vec<_>>()))?;
    let sealed = state.secrets.seal(webhook.url.as_str())?;

    let mut tx = state.db.begin().await?;
    let team_id = body.team_id.flatten();
    if let Some(team_id) = team_id {
        ensure_team(&mut *tx, ctx.organization_id, team_id).await?;
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO integrations (organization_id, team_id, kind, name, enabled, events, mention_users, message_prefix, max_attempts,
                                   secret_ciphertext, secret_nonce, secret_hint, created_by)
         VALUES ($1, $2, 'discord', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) RETURNING id",
    )
    .bind(ctx.organization_id)
    .bind(team_id)
    .bind(name)
    .bind(body.enabled.unwrap_or(true))
    .bind(&events)
    .bind(body.mention_users.unwrap_or(false))
    .bind(body.message_prefix.as_deref().unwrap_or("").trim())
    .bind(body.max_attempts.unwrap_or(6))
    .bind(&sealed.ciphertext)
    .bind(&sealed.nonce)
    .bind(mask(&webhook))
    .bind(ctx.user_id())
    .fetch_one(&mut *tx)
    .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "integration.created",
        "integration",
        Some(id),
        json!({ "kind": "discord", "name": name }),
    )
    .await?;
    tx.commit().await?;
    if !state.secrets.is_encrypting() {
        tracing::warn!("ENCRYPTION_KEY is not set: the Discord webhook URL is stored unencrypted");
    }
    Ok((StatusCode::CREATED, Json(fetch(&state, ctx.organization_id, id).await?)))
}

pub async fn update(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>, Json(body): Json<DiscordInput>) -> AppResult<Json<Integration>> {
    ctx.require(Permission::ManageIntegrations)?;
    validate_common(&body)?;
    fetch(&state, ctx.organization_id, id).await?;
    let events = body.events.as_deref().map(validate_events).transpose()?;
    let secret = match body.webhook_url.as_deref().filter(|u| !u.trim().is_empty()) {
        Some(raw) => {
            let webhook = validate_webhook_url(raw).map_err(|e| AppError::validation("webhook_url", e))?;
            Some((state.secrets.seal(webhook.url.as_str())?, mask(&webhook)))
        }
        None => None,
    };
    let mut tx = state.db.begin().await?;
    if let Some(Some(team_id)) = body.team_id {
        ensure_team(&mut *tx, ctx.organization_id, team_id).await?;
    }
    sqlx::query(
        "UPDATE integrations SET name = COALESCE($2, name), enabled = COALESCE($3, enabled), events = COALESCE($4, events),
            mention_users = COALESCE($5, mention_users), message_prefix = COALESCE($6, message_prefix),
            max_attempts = COALESCE($7, max_attempts), team_id = CASE WHEN $8 THEN $9 ELSE team_id END, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(body.name.as_deref().map(str::trim))
    .bind(body.enabled)
    .bind(&events)
    .bind(body.mention_users)
    .bind(body.message_prefix.as_deref().map(str::trim))
    .bind(body.max_attempts)
    .bind(body.team_id.is_some())
    .bind(body.team_id.flatten())
    .execute(&mut *tx)
    .await?;
    if let Some((sealed, hint)) = &secret {
        sqlx::query("UPDATE integrations SET secret_ciphertext = $2, secret_nonce = $3, secret_hint = $4 WHERE id = $1")
            .bind(id)
            .bind(&sealed.ciphertext)
            .bind(&sealed.nonce)
            .bind(hint)
            .execute(&mut *tx)
            .await?;
    }
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "integration.updated",
        "integration",
        Some(id),
        json!({ "webhook_rotated": secret.is_some() }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(fetch(&state, ctx.organization_id, id).await?))
}

pub async fn delete(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageIntegrations)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("DELETE FROM integrations WHERE id = $1 AND organization_id = $2").bind(id).bind(ctx.organization_id).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "integration.deleted", "integration", Some(id), json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct TestResult {
    delivered: bool,
    status_code: Option<u16>,
    error: Option<String>,
}

/// Send a test message right now and report exactly what Discord answered. The attempt is
/// recorded in the delivery history like any other delivery.
pub async fn test(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<TestResult>> {
    ctx.require(Permission::ManageIntegrations)?;
    let (ciphertext, nonce): (Vec<u8>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT secret_ciphertext, secret_nonce FROM integrations WHERE id = $1 AND organization_id = $2")
            .bind(id)
            .bind(ctx.organization_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or(AppError::NotFound)?;
    let webhook = state.secrets.open(&ciphertext, nonce.as_deref())?;
    let payload = EventPayload { actor_name: Some(ctx.user.display_name.clone()), ..Default::default() };
    let msg =
        message::render(EventKind::Test, &payload, &message::RenderOptions { app_url: state.config.public_url.as_str(), mention_users: false, prefix: "" });
    let started = Utc::now();
    let outcome = state.discord.send(&webhook, &msg).await;
    let (delivered, status_code, error) = match &outcome {
        Outcome::Delivered { status } => (true, Some(*status), None),
        Outcome::Retry { status, error, .. } | Outcome::Failed { status, error } => (false, *status, Some(error.clone())),
    };

    let mut tx = state.db.begin().await?;
    let event_id: Uuid = sqlx::query_scalar(
        "INSERT INTO notification_events (organization_id, event_type, tags, dedupe_key, payload, scheduled_for, status, dispatched_at)
         VALUES ($1, 'test', '{test}', $2, $3, now(), 'dispatched', now()) RETURNING id",
    )
    .bind(ctx.organization_id)
    .bind(format!("test:{}", Uuid::new_v4()))
    .bind(sqlx::types::Json(&payload))
    .fetch_one(&mut *tx)
    .await?;
    let status = if delivered { "succeeded" } else { "failed" };
    let delivery_id: Uuid = sqlx::query_scalar(
        "INSERT INTO notification_deliveries (event_id, integration_id, organization_id, status, attempts, last_status_code, last_error, delivered_at)
         VALUES ($1, $2, $3, $4, 1, $5, $6, CASE WHEN $7 THEN now() END) RETURNING id",
    )
    .bind(event_id)
    .bind(id)
    .bind(ctx.organization_id)
    .bind(status)
    .bind(status_code.map(i32::from))
    .bind(&error)
    .bind(delivered)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO notification_delivery_attempts (delivery_id, attempt, started_at, finished_at, status_code, success, error) VALUES ($1, 1, $2, now(), $3, $4, $5)",
    )
    .bind(delivery_id)
    .bind(started)
    .bind(status_code.map(i32::from))
    .bind(delivered)
    .bind(&error)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE integrations SET last_delivery_status = $2, last_delivery_at = now() WHERE id = $1").bind(id).bind(status).execute(&mut *tx).await?;
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "integration.tested", "integration", Some(id), json!({ "delivered": delivered }))
        .await?;
    tx.commit().await?;
    state.metrics.deliveries.with_label_values(&["discord", status]).inc();
    Ok(Json(TestResult { delivered, status_code, error }))
}
