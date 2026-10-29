//! Shift swaps: a member asks a colleague to cover one of their shifts, optionally taking one of
//! the colleague's shifts in exchange. When the colleague accepts, the swap becomes ordinary
//! overrides, so the rotation engine, notifications and history need nothing new.
//!
//! The schedule may change while a request waits, so everything is checked twice: when the
//! request is made and again, inside the accepting transaction, when it is accepted. A request
//! expires once either of its shifts has begun.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::{ensure_members, OrgContext},
    error::{AppError, AppResult},
    notifications::planner,
    rbac::Permission,
    schedules::{self, engine::Layer, repo, NewOverride},
    state::AppState,
};

/// The longest window one swap can cover, matching overrides.
const MAX_WINDOW: Duration = Duration::days(90);
/// Open requests one person may have at a time.
const MAX_PENDING: i64 = 20;

#[derive(Serialize, sqlx::FromRow)]
pub struct Swap {
    id: Uuid,
    schedule_id: Uuid,
    schedule_name: String,
    timezone: String,
    layer: String,
    requester_id: Uuid,
    target_user_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    return_start_at: Option<DateTime<Utc>>,
    return_end_at: Option<DateTime<Utc>>,
    reason: String,
    /// pending, accepted, declined, canceled, or expired (a pending request whose shift has
    /// begun, or whose schedule was archived).
    status: String,
    created_at: DateTime<Utc>,
    decided_at: Option<DateTime<Utc>>,
    decided_by: Option<Uuid>,
}

const SWAP_SELECT: &str = "SELECT w.id, w.schedule_id, s.name AS schedule_name, s.timezone, w.layer, w.requester_id, w.target_user_id,
        w.start_at, w.end_at, w.return_start_at, w.return_end_at, w.reason,
        CASE WHEN w.status = 'pending' AND (LEAST(w.start_at, COALESCE(w.return_start_at, w.start_at)) <= now() OR s.status = 'archived')
             THEN 'expired' ELSE w.status END AS status,
        w.created_at, w.decided_at, w.decided_by
     FROM shift_swaps w JOIN schedules s ON s.id = w.schedule_id";

#[derive(Deserialize)]
pub struct ListQuery {
    /// Every request in the organization (schedulers and above). Otherwise only requests you
    /// made or received.
    #[serde(default)]
    all: bool,
}

pub async fn list(State(state): State<AppState>, ctx: OrgContext, Query(q): Query<ListQuery>) -> AppResult<Json<Vec<Swap>>> {
    if q.all {
        ctx.require(Permission::ManageSchedules)?;
    }
    let swaps = sqlx::query_as(&format!(
        "{SWAP_SELECT} WHERE w.organization_id = $1 AND ($2 OR w.requester_id = $3 OR w.target_user_id = $3)
         ORDER BY w.created_at DESC LIMIT 200"
    ))
    .bind(ctx.organization_id)
    .bind(q.all)
    .bind(ctx.user_id())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(swaps))
}

#[derive(Deserialize)]
pub struct CreateSwap {
    schedule_id: Uuid,
    layer: Layer,
    target_user_id: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    /// One of the target's shifts the requester takes in exchange. Both or neither.
    return_start: Option<DateTime<Utc>>,
    return_end: Option<DateTime<Utc>>,
    #[serde(default)]
    reason: String,
}

fn validate_window(field: &str, start: DateTime<Utc>, end: DateTime<Utc>, now: DateTime<Utc>) -> AppResult<()> {
    if end <= start {
        return Err(AppError::validation(field, "the end must be after the start"));
    }
    if start <= now {
        return Err(AppError::validation(field, "only shifts that have not started yet can be swapped"));
    }
    if end - start > MAX_WINDOW {
        return Err(AppError::validation(field, "a swap can cover at most 90 days"));
    }
    Ok(())
}

/// Whether the schedule, as it stands now, still allows this swap.
fn check_coverage(
    loaded: &repo::LoadedSchedule,
    layer: Layer,
    requester: Uuid,
    target: Uuid,
    window: (DateTime<Utc>, DateTime<Utc>),
    ret: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> AppResult<()> {
    if loaded.row.status == "archived" {
        return Err(AppError::Conflict("this schedule is archived".into()));
    }
    if !loaded.spec.holds(layer, requester, window.0, window.1) {
        return Err(AppError::Conflict("the person asking is not on call for that whole shift (any more); the schedule may have changed".into()));
    }
    if let Some((start, end)) = ret {
        if !loaded.spec.holds(layer, target, start, end) {
            return Err(AppError::Conflict("the shift offered in exchange is not the colleague's (any more); the schedule may have changed".into()));
        }
    }
    Ok(())
}

pub async fn create(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<CreateSwap>) -> AppResult<(StatusCode, Json<Swap>)> {
    let now = Utc::now();
    if body.target_user_id == ctx.user_id() {
        return Err(AppError::validation("target_user_id", "choose someone other than yourself"));
    }
    validate_window("start", body.start, body.end, now)?;
    let ret = match (body.return_start, body.return_end) {
        (Some(start), Some(end)) => {
            validate_window("return_start", start, end, now)?;
            if start < body.end && body.start < end {
                return Err(AppError::validation("return_start", "the shift taken in exchange cannot overlap the shift being handed over"));
            }
            Some((start, end))
        }
        (None, None) => None,
        _ => return Err(AppError::validation("return_start", "give both the start and the end of the shift taken in exchange")),
    };
    let reason = body.reason.trim();
    if reason.chars().count() > 500 {
        return Err(AppError::validation("reason", "keep the reason under 500 characters"));
    }

    let loaded = repo::load_one(&state.db, ctx.organization_id, body.schedule_id).await.map_err(|e| match e {
        AppError::NotFound => AppError::validation("schedule_id", "schedule not found"),
        other => other,
    })?;
    check_coverage(&loaded, body.layer, ctx.user_id(), body.target_user_id, (body.start, body.end), ret)?;

    let mut tx = state.db.begin().await?;
    ensure_members(&mut tx, ctx.organization_id, &[body.target_user_id]).await?;
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM shift_swaps WHERE requester_id = $1 AND organization_id = $2 AND status = 'pending' AND start_at > now()")
            .bind(ctx.user_id())
            .bind(ctx.organization_id)
            .fetch_one(&mut *tx)
            .await?;
    if pending >= MAX_PENDING {
        return Err(AppError::Conflict(format!("you have {MAX_PENDING} open swap requests; wait for answers or cancel some first")));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO shift_swaps (organization_id, schedule_id, layer, requester_id, target_user_id, start_at, end_at, return_start_at, return_end_at, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(ctx.organization_id)
    .bind(body.schedule_id)
    .bind(body.layer.as_str())
    .bind(ctx.user_id())
    .bind(body.target_user_id)
    .bind(body.start)
    .bind(body.end)
    .bind(ret.map(|r| r.0))
    .bind(ret.map(|r| r.1))
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "swap.requested",
        "schedule",
        Some(body.schedule_id),
        json!({ "swap_id": id, "target_user_id": body.target_user_id, "layer": body.layer.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(fetch(&state, ctx.organization_id, id).await?)))
}

async fn fetch(state: &AppState, organization_id: Uuid, id: Uuid) -> AppResult<Swap> {
    sqlx::query_as(&format!("{SWAP_SELECT} WHERE w.id = $1 AND w.organization_id = $2"))
        .bind(id)
        .bind(organization_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)
}

#[derive(sqlx::FromRow)]
struct PendingSwap {
    schedule_id: Uuid,
    layer: String,
    requester_id: Uuid,
    target_user_id: Uuid,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    return_start_at: Option<DateTime<Utc>>,
    return_end_at: Option<DateTime<Utc>>,
    reason: String,
}

/// Lock a request that is still pending and has not expired.
async fn lock_pending(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, organization_id: Uuid, id: Uuid) -> AppResult<PendingSwap> {
    let swap: Option<(PendingSwap, String)> = sqlx::query_as::<_, PendingSwapWithStatus>(
        "SELECT schedule_id, layer, requester_id, target_user_id, start_at, end_at, return_start_at, return_end_at, reason, status
         FROM shift_swaps WHERE id = $1 AND organization_id = $2 FOR UPDATE",
    )
    .bind(id)
    .bind(organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .map(|r| (r.swap, r.status));
    let (swap, status) = swap.ok_or(AppError::NotFound)?;
    if status != "pending" {
        return Err(AppError::Conflict(format!("this request was already {status}")));
    }
    if swap.start_at.min(swap.return_start_at.unwrap_or(swap.start_at)) <= Utc::now() {
        return Err(AppError::Conflict("this request expired: its shift has already begun".into()));
    }
    Ok(swap)
}

#[derive(sqlx::FromRow)]
struct PendingSwapWithStatus {
    #[sqlx(flatten)]
    swap: PendingSwap,
    status: String,
}

pub async fn accept(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<Swap>> {
    let mut tx = state.db.begin().await?;
    let swap = lock_pending(&mut tx, ctx.organization_id, id).await?;
    if swap.target_user_id != ctx.user_id() {
        return Err(AppError::Forbidden("only the person who was asked can accept".into()));
    }
    // Serialise against other changes to this schedule, then check against its current state.
    sqlx::query("SELECT id FROM schedules WHERE id = $1 FOR UPDATE").bind(swap.schedule_id).execute(&mut *tx).await?;
    let loaded = repo::load_one(&state.db, ctx.organization_id, swap.schedule_id).await?;
    let layer = Layer::parse(&swap.layer).ok_or_else(|| AppError::Internal(anyhow::anyhow!("invalid layer in swap {id}")))?;
    let ret = swap.return_start_at.zip(swap.return_end_at);
    check_coverage(&loaded, layer, swap.requester_id, swap.target_user_id, (swap.start_at, swap.end_at), ret)?;
    ensure_members(&mut tx, ctx.organization_id, &[swap.requester_id, swap.target_user_id]).await?;

    let names = planner::load_people(&mut tx, &[swap.requester_id, swap.target_user_id]).await?;
    let name = |id: Uuid| names.get(&id).map(|p| p.name.clone()).unwrap_or_else(|| "a colleague".into());
    let note = |with: Uuid| {
        let base = format!("Swap with {}", name(with));
        let reason: String = if swap.reason.is_empty() { base } else { format!("{base}: {}", swap.reason) };
        reason.chars().take(500).collect::<String>()
    };
    let covering = schedules::insert_override(
        &mut tx,
        &loaded,
        NewOverride { layer, user_id: swap.target_user_id, start: swap.start_at, end: swap.end_at, reason: &note(swap.requester_id) },
        ctx.user_id(),
        &ctx.user.display_name,
    )
    .await?;
    let mut override_ids = vec![covering.id];
    if let Some((start, end)) = ret {
        let back = schedules::insert_override(
            &mut tx,
            &loaded,
            NewOverride { layer, user_id: swap.requester_id, start, end, reason: &note(swap.target_user_id) },
            ctx.user_id(),
            &ctx.user.display_name,
        )
        .await?;
        override_ids.push(back.id);
    }
    sqlx::query("UPDATE shift_swaps SET status = 'accepted', decided_at = now(), decided_by = $2 WHERE id = $1")
        .bind(id)
        .bind(ctx.user_id())
        .execute(&mut *tx)
        .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "swap.accepted",
        "schedule",
        Some(swap.schedule_id),
        json!({ "swap_id": id, "override_ids": override_ids }),
    )
    .await?;
    tx.commit().await?;
    planner::plan_schedule_by_id(&state, swap.schedule_id).await.ok();
    Ok(Json(fetch(&state, ctx.organization_id, id).await?))
}

/// Decline (the person asked) or cancel (the person asking, or a scheduler).
async fn close(state: &AppState, ctx: &OrgContext, id: Uuid, decline: bool) -> AppResult<Json<Swap>> {
    let mut tx = state.db.begin().await?;
    let swap: Option<(Uuid, Uuid, Uuid, String)> =
        sqlx::query_as("SELECT schedule_id, requester_id, target_user_id, status FROM shift_swaps WHERE id = $1 AND organization_id = $2 FOR UPDATE")
            .bind(id)
            .bind(ctx.organization_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (schedule_id, requester, target, status) = swap.ok_or(AppError::NotFound)?;
    let allowed = if decline { target == ctx.user_id() } else { requester == ctx.user_id() || ctx.role.allows(Permission::ManageSchedules) };
    if !allowed {
        return Err(AppError::Forbidden(
            if decline { "only the person who was asked can decline" } else { "only the person who asked, or a scheduler, can cancel" }.into(),
        ));
    }
    if status != "pending" {
        return Err(AppError::Conflict(format!("this request was already {status}")));
    }
    let new_status = if decline { "declined" } else { "canceled" };
    sqlx::query("UPDATE shift_swaps SET status = $2, decided_at = now(), decided_by = $3 WHERE id = $1")
        .bind(id)
        .bind(new_status)
        .bind(ctx.user_id())
        .execute(&mut *tx)
        .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        &format!("swap.{new_status}"),
        "schedule",
        Some(schedule_id),
        json!({ "swap_id": id }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(fetch(state, ctx.organization_id, id).await?))
}

pub async fn decline(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<Swap>> {
    close(&state, &ctx, id, true).await
}

pub async fn cancel(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<Swap>> {
    close(&state, &ctx, id, false).await
}
