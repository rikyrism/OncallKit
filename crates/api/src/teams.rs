use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::{ensure_members, OrgContext},
    error::{AppError, AppResult},
    rbac::Permission,
    state::AppState,
};

#[derive(Serialize, sqlx::FromRow)]
pub struct Team {
    id: Uuid,
    name: String,
    description: String,
    created_at: DateTime<Utc>,
    member_ids: Vec<Uuid>,
    active_schedule_count: i64,
}

const TEAM_SELECT: &str = "SELECT t.id, t.name, t.description, t.created_at,
        COALESCE((SELECT array_agg(tm.user_id ORDER BY tm.created_at) FROM team_memberships tm WHERE tm.team_id = t.id), '{}') AS member_ids,
        (SELECT count(*) FROM schedules s WHERE s.team_id = t.id AND s.status = 'active') AS active_schedule_count
     FROM teams t";

/// Fetch a team only if it belongs to the organization; otherwise it does not exist.
pub async fn ensure_team(db: impl sqlx::PgExecutor<'_>, organization_id: Uuid, team_id: Uuid) -> AppResult<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM teams WHERE id = $1 AND organization_id = $2)")
        .bind(team_id)
        .bind(organization_id)
        .fetch_one(db)
        .await?;
    if exists {
        Ok(())
    } else {
        Err(AppError::validation("team_id", "team not found"))
    }
}

pub async fn list(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Vec<Team>>> {
    let teams =
        sqlx::query_as(&format!("{TEAM_SELECT} WHERE t.organization_id = $1 ORDER BY lower(t.name)")).bind(ctx.organization_id).fetch_all(&state.db).await?;
    Ok(Json(teams))
}

pub async fn get(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<Team>> {
    let team = sqlx::query_as(&format!("{TEAM_SELECT} WHERE t.organization_id = $1 AND t.id = $2"))
        .bind(ctx.organization_id)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(team))
}

#[derive(Deserialize)]
pub struct TeamInput {
    name: Option<String>,
    description: Option<String>,
    member_ids: Option<Vec<Uuid>>,
}

fn validate_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::validation("name", "use 1 to 120 characters"));
    }
    Ok(name)
}

async fn set_members(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: Uuid, team_id: Uuid, member_ids: &[Uuid]) -> AppResult<()> {
    ensure_members(tx, org, member_ids).await?;
    sqlx::query("DELETE FROM team_memberships WHERE team_id = $1 AND NOT (user_id = ANY($2))").bind(team_id).bind(member_ids).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO team_memberships (team_id, user_id) SELECT $1, unnest($2::uuid[]) ON CONFLICT DO NOTHING")
        .bind(team_id)
        .bind(member_ids)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn create(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<TeamInput>) -> AppResult<(StatusCode, Json<Team>)> {
    ctx.require(Permission::ManageTeams)?;
    let name = validate_name(body.name.as_deref().unwrap_or(""))?;
    let mut tx = state.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO teams (organization_id, name, description) VALUES ($1, $2, $3) RETURNING id")
        .bind(ctx.organization_id)
        .bind(name)
        .bind(body.description.as_deref().unwrap_or("").trim())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match AppError::from(e) {
            AppError::Conflict(_) => AppError::validation("name", "a team with this name already exists"),
            other => other,
        })?;
    if let Some(members) = &body.member_ids {
        set_members(&mut tx, ctx.organization_id, id, members).await?;
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "team.created", "team", Some(id), json!({ "name": name })).await?;
    tx.commit().await?;
    let team = get(State(state), ctx, Path(id)).await?;
    Ok((StatusCode::CREATED, team))
}

pub async fn update(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>, Json(body): Json<TeamInput>) -> AppResult<Json<Team>> {
    ctx.require(Permission::ManageTeams)?;
    let mut tx = state.db.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM teams WHERE id = $1 AND organization_id = $2)")
        .bind(id)
        .bind(ctx.organization_id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(AppError::NotFound);
    }
    let name = body.name.as_deref().map(validate_name).transpose()?;
    sqlx::query("UPDATE teams SET name = COALESCE($2, name), description = COALESCE($3, description), updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(body.description.as_deref().map(str::trim))
        .execute(&mut *tx)
        .await?;
    if let Some(members) = &body.member_ids {
        set_members(&mut tx, ctx.organization_id, id, members).await?;
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "team.updated", "team", Some(id), json!({})).await?;
    tx.commit().await?;
    get(State(state), ctx, Path(id)).await
}

pub async fn delete(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageTeams)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("DELETE FROM teams WHERE id = $1 AND organization_id = $2").bind(id).bind(ctx.organization_id).execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "team.deleted", "team", Some(id), json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
