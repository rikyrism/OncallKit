use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    auth::OrgContext,
    error::AppResult,
    pagination::{Page, PageQuery},
    rbac::Permission,
    state::AppState,
};

pub async fn record_tx(
    tx: &mut Transaction<'_, Postgres>,
    organization_id: Option<Uuid>,
    actor_user_id: Option<Uuid>,
    action: &str,
    target_type: &str,
    target_id: Option<Uuid>,
    metadata: serde_json::Value,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO audit_logs (organization_id, actor_user_id, action, target_type, target_id, metadata) VALUES ($1, $2, $3, $4, $5, $6)")
        .bind(organization_id)
        .bind(actor_user_id)
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(metadata)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct AuditEntry {
    id: Uuid,
    action: String,
    target_type: String,
    target_id: Option<Uuid>,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
    actor_user_id: Option<Uuid>,
    actor_name: Option<String>,
}

#[derive(Deserialize)]
pub struct AuditQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    action: Option<String>,
}

pub async fn list(State(state): State<AppState>, ctx: OrgContext, Query(q): Query<AuditQuery>) -> AppResult<Json<Page<AuditEntry>>> {
    ctx.require(Permission::ViewAuditLogs)?;
    let (limit, offset) = PageQuery { limit: q.limit, offset: q.offset }.limit_offset();
    let action = q.action.filter(|a| !a.is_empty()).map(|a| format!("{}%", a.replace('%', "")));
    let items: Vec<AuditEntry> = sqlx::query_as(
        "SELECT a.id, a.action, a.target_type, a.target_id, a.metadata, a.created_at, a.actor_user_id, u.display_name AS actor_name
         FROM audit_logs a LEFT JOIN users u ON u.id = a.actor_user_id
         WHERE a.organization_id = $1 AND ($2::text IS NULL OR a.action LIKE $2)
         ORDER BY a.created_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(ctx.organization_id)
    .bind(&action)
    .bind(limit + 1)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(Page::new(items, limit, offset)))
}
