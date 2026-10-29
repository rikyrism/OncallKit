use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::OrgContext,
    error::{AppError, AppResult},
    pagination::{Page, PageQuery},
    rbac::Permission,
    state::AppState,
};

#[derive(Serialize, sqlx::FromRow)]
pub struct DeliveryView {
    id: Uuid,
    status: String,
    attempts: i32,
    max_attempts: i32,
    next_attempt_at: DateTime<Utc>,
    last_status_code: Option<i32>,
    last_error: Option<String>,
    delivered_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    event_type: String,
    event_scheduled_for: DateTime<Utc>,
    schedule_id: Option<Uuid>,
    schedule_name: Option<String>,
    integration_id: Uuid,
    integration_name: String,
}

#[derive(Deserialize)]
pub struct DeliveryQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    status: Option<String>,
    integration_id: Option<Uuid>,
}

pub async fn list_deliveries(State(state): State<AppState>, ctx: OrgContext, Query(q): Query<DeliveryQuery>) -> AppResult<Json<Page<DeliveryView>>> {
    ctx.require(Permission::ViewIntegrations)?;
    let (limit, offset) = PageQuery { limit: q.limit, offset: q.offset }.limit_offset();
    let status = q.status.filter(|s| !s.is_empty() && s != "all");
    let items = sqlx::query_as(
        "SELECT d.id, d.status, d.attempts, i.max_attempts, d.next_attempt_at, d.last_status_code, d.last_error, d.delivered_at,
                d.created_at, d.updated_at, e.event_type, e.scheduled_for AS event_scheduled_for, e.schedule_id,
                e.payload->>'schedule_name' AS schedule_name, d.integration_id, i.name AS integration_name
         FROM notification_deliveries d
         JOIN notification_events e ON e.id = d.event_id
         JOIN integrations i ON i.id = d.integration_id
         WHERE d.organization_id = $1 AND ($2::text IS NULL OR d.status = $2) AND ($3::uuid IS NULL OR d.integration_id = $3)
         ORDER BY d.created_at DESC LIMIT $4 OFFSET $5",
    )
    .bind(ctx.organization_id)
    .bind(&status)
    .bind(q.integration_id)
    .bind(limit + 1)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Page::new(items, limit, offset)))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AttemptView {
    attempt: i32,
    started_at: DateTime<Utc>,
    finished_at: DateTime<Utc>,
    status_code: Option<i32>,
    success: bool,
    error: Option<String>,
}

pub async fn delivery_attempts(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<Vec<AttemptView>>> {
    ctx.require(Permission::ViewIntegrations)?;
    let items = sqlx::query_as(
        "SELECT a.attempt, a.started_at, a.finished_at, a.status_code, a.success, a.error FROM notification_delivery_attempts a
         JOIN notification_deliveries d ON d.id = a.delivery_id WHERE d.id = $1 AND d.organization_id = $2 ORDER BY a.attempt",
    )
    .bind(id)
    .bind(ctx.organization_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(items))
}

/// Put a failed delivery back in the queue.
pub async fn retry_delivery(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageIntegrations)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query(
        "UPDATE notification_deliveries SET status = 'pending', attempts = 0, next_attempt_at = now(), updated_at = now()
         WHERE id = $1 AND organization_id = $2 AND status = 'failed'",
    )
    .bind(id)
    .bind(ctx.organization_id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::Conflict("only failed deliveries can be retried".into()));
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "delivery.retried", "delivery", Some(id), json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct Health {
    succeeded_24h: i64,
    failed_24h: i64,
    pending: i64,
    retrying: i64,
    integrations: i64,
    integrations_enabled: i64,
    last_success_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
}

/// Notification health for the dashboard. Members see it too: it contains no secrets.
pub async fn health(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Health>> {
    let (succeeded_24h, failed_24h, pending, retrying, last_success_at, last_failure_at): (i64, i64, i64, i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as(
            "SELECT count(*) FILTER (WHERE status = 'succeeded' AND updated_at > now() - interval '24 hours'),
                count(*) FILTER (WHERE status = 'failed' AND updated_at > now() - interval '24 hours'),
                count(*) FILTER (WHERE status IN ('pending', 'sending') AND attempts = 0),
                count(*) FILTER (WHERE status IN ('pending', 'sending') AND attempts > 0),
                max(delivered_at) FILTER (WHERE status = 'succeeded'),
                max(updated_at) FILTER (WHERE status = 'failed')
         FROM notification_deliveries WHERE organization_id = $1",
        )
        .bind(ctx.organization_id)
        .fetch_one(&state.db)
        .await?;
    let (integrations, integrations_enabled): (i64, i64) =
        sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE enabled) FROM integrations WHERE organization_id = $1")
            .bind(ctx.organization_id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(Health { succeeded_24h, failed_24h, pending, retrying, integrations, integrations_enabled, last_success_at, last_failure_at }))
}
