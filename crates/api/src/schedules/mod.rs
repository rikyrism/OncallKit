//! Schedules: HTTP handlers over the rotation [`engine`] and the versioned storage in [`repo`].

pub mod engine;
pub mod repo;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use self::{
    engine::{Layer, RotationUnit, RotationVersion, ScheduleSpec, Shift},
    repo::{LoadedSchedule, OverrideRow, ScheduleRow, VersionView},
};
use crate::{
    audit,
    auth::{ensure_members, OrgContext},
    error::{AppError, AppResult},
    notifications::{self, planner, EventKind, EventPayload, NewEvent},
    organizations::validate_timezone,
    rbac::Permission,
    state::AppState,
    teams::ensure_team,
};

// ---------- views ----------

#[derive(Serialize)]
pub struct OnCallNow {
    pub at: DateTime<Utc>,
    pub primary: Option<Shift>,
    pub secondary: Option<Shift>,
    pub next_rotation_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct ScheduleView {
    #[serde(flatten)]
    pub row: ScheduleRow,
    pub start_local: String,
    pub end_local: Option<String>,
    /// not_started, active, ended, paused or archived.
    pub state: &'static str,
    pub rotation: Option<VersionView>,
    pub on_call: OnCallNow,
}

#[derive(Serialize)]
pub struct ScheduleDetail {
    #[serde(flatten)]
    pub schedule: ScheduleView,
    pub versions: Vec<VersionView>,
    pub overrides: Vec<OverrideRow>,
}

pub fn local_string(t: DateTime<Utc>, tz: Tz) -> String {
    t.with_timezone(&tz).format("%Y-%m-%dT%H:%M").to_string()
}

pub fn on_call_now(spec: &ScheduleSpec, archived: bool, at: DateTime<Utc>) -> OnCallNow {
    if archived {
        return OnCallNow { at, primary: None, secondary: None, next_rotation_at: None };
    }
    OnCallNow {
        at,
        primary: spec.on_call_at(Layer::Primary, at),
        secondary: spec.on_call_at(Layer::Secondary, at),
        next_rotation_at: spec.next_rotation_boundary(Layer::Primary, at),
    }
}

fn view(loaded: &LoadedSchedule, now: DateTime<Utc>) -> ScheduleView {
    let row = &loaded.row;
    let tz = row.tz();
    let state = match row.status.as_str() {
        "archived" => "archived",
        "paused" => "paused",
        _ if now < row.start_at => "not_started",
        _ if row.end_at.is_some_and(|e| now >= e) => "ended",
        _ => "active",
    };
    let rotation = loaded.versions.iter().rfind(|v| v.effective_from <= now).or_else(|| loaded.versions.first()).cloned();
    ScheduleView {
        start_local: local_string(row.start_at, tz),
        end_local: row.end_at.map(|e| local_string(e, tz)),
        state,
        rotation,
        on_call: on_call_now(&loaded.spec, row.status == "archived", now),
        row: row.clone(),
    }
}

// ---------- input ----------

#[derive(Clone, Deserialize)]
pub struct RotationInput {
    pub interval: u32,
    pub unit: RotationUnit,
    pub primary: Vec<Uuid>,
    #[serde(default)]
    pub secondary: Vec<Uuid>,
}

#[derive(Deserialize)]
pub struct ScheduleInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub team_id: Option<Uuid>,
    pub timezone: String,
    /// Local wall-clock start in the schedule's timezone, `YYYY-MM-DDTHH:MM`.
    pub start_local: String,
    /// Local wall-clock end, or null/absent for an open-ended schedule.
    pub end_local: Option<String>,
    pub rotation: RotationInput,
    pub reminder_offsets_minutes: Option<Vec<i32>>,
    pub notify_on_boundary: Option<bool>,
}

pub fn parse_local(field: &str, value: &str, tz: Tz) -> AppResult<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| AppError::validation(field, "use the format YYYY-MM-DDTHH:MM"))?;
    Ok(engine::resolve_local(tz, naive))
}

fn validate_rotation(rotation: &RotationInput) -> AppResult<()> {
    let max = match rotation.unit {
        RotationUnit::Hours => 24 * 365,
        RotationUnit::Days => 365,
        RotationUnit::Weeks => 52,
    };
    if rotation.interval == 0 || rotation.interval > max {
        return Err(AppError::validation("rotation.interval", format!("choose between 1 and {max}")));
    }
    if rotation.primary.len() > 100 || rotation.secondary.len() > 100 {
        return Err(AppError::validation("rotation.primary", "a rotation can have at most 100 people"));
    }
    let version = RotationVersion {
        effective_from: Utc::now(),
        interval: rotation.interval,
        unit: rotation.unit,
        primary: rotation.primary.clone(),
        secondary: rotation.secondary.clone(),
    };
    engine::validate_version(&version).map_err(|e| AppError::validation("rotation.primary", e.to_string()))
}

fn validate_offsets(offsets: &[i32]) -> AppResult<Vec<i32>> {
    if offsets.len() > 5 || offsets.iter().any(|o| *o < 1 || *o > 7 * 24 * 60) {
        return Err(AppError::validation("reminder_offsets_minutes", "up to five reminders, each between 1 minute and 7 days before"));
    }
    let mut v = offsets.to_vec();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v.dedup();
    Ok(v)
}

fn validate_name(name: &str) -> AppResult<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::validation("name", "use 1 to 120 characters"));
    }
    Ok(name)
}

struct ValidatedTimes {
    tz: Tz,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
}

fn validate_times(timezone: &str, start_local: &str, end_local: Option<&str>) -> AppResult<ValidatedTimes> {
    let tz = validate_timezone(timezone)?;
    let start = parse_local("start_local", start_local, tz)?;
    let end = end_local.filter(|e| !e.trim().is_empty()).map(|e| parse_local("end_local", e, tz)).transpose()?;
    if let Some(end) = end {
        if end <= start {
            return Err(AppError::validation("end_local", "the end must be after the start"));
        }
    }
    Ok(ValidatedTimes { tz, start, end })
}

// ---------- handlers ----------

#[derive(Deserialize)]
pub struct ListQuery {
    team_id: Option<Uuid>,
    #[serde(default)]
    include_archived: bool,
}

pub async fn list(State(state): State<AppState>, ctx: OrgContext, Query(q): Query<ListQuery>) -> AppResult<Json<Vec<ScheduleView>>> {
    let mut loaded = repo::load_many(&state.db, ctx.organization_id, if q.include_archived { "" } else { "AND status <> 'archived'" }).await?;
    if let Some(team) = q.team_id {
        loaded.retain(|l| l.row.team_id == Some(team));
    }
    let now = Utc::now();
    Ok(Json(loaded.iter().map(|l| view(l, now)).collect()))
}

pub async fn get(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<Json<ScheduleDetail>> {
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    let now = Utc::now();
    Ok(Json(ScheduleDetail { schedule: view(&loaded, now), versions: loaded.versions.clone(), overrides: loaded.overrides.clone() }))
}

#[derive(Serialize)]
pub struct Preview {
    pub start_at: DateTime<Utc>,
    pub end_at: Option<DateTime<Utc>>,
    pub shifts: Vec<Shift>,
}

/// Show the shifts a configuration would produce, without saving anything.
pub async fn preview(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<ScheduleInput>) -> AppResult<Json<Preview>> {
    validate_rotation(&body.rotation)?;
    let times = validate_times(&body.timezone, &body.start_local, body.end_local.as_deref())?;
    let mut tx = state.db.begin().await?;
    ensure_members(&mut tx, ctx.organization_id, &[body.rotation.primary.clone(), body.rotation.secondary.clone()].concat()).await?;
    tx.rollback().await?;
    let spec = ScheduleSpec::new(
        times.tz,
        times.start,
        times.end,
        vec![RotationVersion {
            effective_from: times.start,
            interval: body.rotation.interval,
            unit: body.rotation.unit,
            primary: body.rotation.primary,
            secondary: body.rotation.secondary,
        }],
        vec![],
    );
    let from = times.start.max(Utc::now() - Duration::days(1));
    let mut shifts = spec.shifts(from, from + Duration::days(120));
    shifts.truncate(60);
    Ok(Json(Preview { start_at: times.start, end_at: times.end, shifts }))
}

pub async fn create(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<ScheduleInput>) -> AppResult<(StatusCode, Json<ScheduleDetail>)> {
    ctx.require(Permission::ManageSchedules)?;
    let name = validate_name(&body.name)?;
    validate_rotation(&body.rotation)?;
    let times = validate_times(&body.timezone, &body.start_local, body.end_local.as_deref())?;
    let offsets = validate_offsets(body.reminder_offsets_minutes.as_deref().unwrap_or(&[1440, 60]))?;

    let mut tx = state.db.begin().await?;
    if let Some(team_id) = body.team_id {
        ensure_team(&mut *tx, ctx.organization_id, team_id).await?;
    }
    ensure_members(&mut tx, ctx.organization_id, &[body.rotation.primary.clone(), body.rotation.secondary.clone()].concat()).await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO schedules (organization_id, team_id, name, description, timezone, start_at, end_at, reminder_offsets_minutes, notify_on_boundary, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(ctx.organization_id)
    .bind(body.team_id)
    .bind(name)
    .bind(body.description.trim())
    .bind(times.tz.name())
    .bind(times.start)
    .bind(times.end)
    .bind(&offsets)
    .bind(body.notify_on_boundary.unwrap_or(true))
    .bind(ctx.user_id())
    .fetch_one(&mut *tx)
    .await?;
    repo::insert_version(
        &mut tx,
        id,
        times.start,
        body.rotation.interval,
        body.rotation.unit,
        &body.rotation.primary,
        &body.rotation.secondary,
        Some(ctx.user_id()),
    )
    .await?;
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "schedule.created", "schedule", Some(id), json!({ "name": name })).await?;
    let payload = EventPayload {
        schedule_id: Some(id),
        schedule_name: Some(name.into()),
        timezone: Some(times.tz.name().into()),
        at: Some(times.start),
        actor_name: Some(ctx.user.display_name.clone()),
        ..Default::default()
    };
    notifications::enqueue(
        &mut tx,
        NewEvent {
            organization_id: ctx.organization_id,
            schedule_id: Some(id),
            team_id: body.team_id,
            kind: EventKind::ScheduleCreated,
            tags: vec![],
            dedupe_key: format!("schedule_created:{id}"),
            payload: &payload,
            scheduled_for: Utc::now(),
            exclude_integration_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    planner::plan_schedule_by_id(&state, id).await.ok();
    let detail = get(State(state), ctx, Path(id)).await?;
    Ok((StatusCode::CREATED, detail))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyFrom {
    /// The new rotation takes over immediately.
    Now,
    /// The current shift finishes as planned; the new rotation starts at the next hand-over.
    NextRotation,
}

#[derive(Deserialize)]
pub struct ScheduleUpdate {
    name: Option<String>,
    description: Option<String>,
    /// Use `null` to detach from a team. Omit to leave unchanged.
    #[serde(default, deserialize_with = "double_option")]
    team_id: Option<Option<Uuid>>,
    status: Option<String>,
    timezone: Option<String>,
    start_local: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    end_local: Option<Option<String>>,
    rotation: Option<RotationInput>,
    apply_from: Option<ApplyFrom>,
    reminder_offsets_minutes: Option<Vec<i32>>,
    notify_on_boundary: Option<bool>,
}

fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

pub async fn update(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>, Json(body): Json<ScheduleUpdate>) -> AppResult<Json<ScheduleDetail>> {
    ctx.require(Permission::ManageSchedules)?;
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    let row = &loaded.row;
    let now = Utc::now();
    let started = now >= row.start_at;

    let name = body.name.as_deref().map(validate_name).transpose()?;
    if let Some(status) = &body.status {
        if !["active", "paused", "archived"].contains(&status.as_str()) {
            return Err(AppError::validation("status", "use active, paused or archived"));
        }
    }
    let offsets = body.reminder_offsets_minutes.as_deref().map(validate_offsets).transpose()?;

    // Timezone and start define every past shift, so they are frozen once the schedule starts.
    let timezone = body.timezone.clone().unwrap_or_else(|| row.timezone.clone());
    let start_local = body.start_local.clone().unwrap_or_else(|| local_string(row.start_at, row.tz()));
    if started && (timezone != row.timezone || body.start_local.as_ref().is_some_and(|s| parse_local("start_local", s, row.tz()).ok() != Some(row.start_at))) {
        return Err(AppError::validation("start_local", "the start and timezone cannot change after the schedule has started; create a new schedule instead"));
    }
    let end_local: Option<String> = match &body.end_local {
        Some(v) => v.clone(),
        None => row.end_at.map(|e| local_string(e, row.tz())),
    };
    let times = validate_times(&timezone, &start_local, end_local.as_deref())?;
    if let (Some(new_end), Some(_)) = (times.end, &body.end_local) {
        if new_end < now && row.end_at.is_none_or(|old| old > now) {
            return Err(AppError::validation("end_local", "the end cannot be moved into the past"));
        }
    }

    let mut tx = state.db.begin().await?;
    if let Some(Some(team_id)) = body.team_id {
        ensure_team(&mut *tx, ctx.organization_id, team_id).await?;
    }
    sqlx::query(
        "UPDATE schedules SET name = COALESCE($2, name), description = COALESCE($3, description),
            team_id = CASE WHEN $4 THEN $5 ELSE team_id END, status = COALESCE($6, status),
            timezone = $7, start_at = $8, end_at = $9,
            reminder_offsets_minutes = COALESCE($10, reminder_offsets_minutes),
            notify_on_boundary = COALESCE($11, notify_on_boundary), updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(body.description.as_deref().map(str::trim))
    .bind(body.team_id.is_some())
    .bind(body.team_id.flatten())
    .bind(&body.status)
    .bind(times.tz.name())
    .bind(times.start)
    .bind(times.end)
    .bind(&offsets)
    .bind(body.notify_on_boundary)
    .execute(&mut *tx)
    .await?;

    let mut change = json!({});
    if let Some(rotation) = &body.rotation {
        validate_rotation(rotation)?;
        ensure_members(&mut tx, ctx.organization_id, &[rotation.primary.clone(), rotation.secondary.clone()].concat()).await?;
        if !started {
            // Nothing has happened yet: replace the configuration outright.
            sqlx::query("DELETE FROM schedule_versions WHERE schedule_id = $1").bind(id).execute(&mut *tx).await?;
            repo::insert_version(&mut tx, id, times.start, rotation.interval, rotation.unit, &rotation.primary, &rotation.secondary, Some(ctx.user_id()))
                .await?;
            change = json!({ "rotation": "replaced" });
        } else {
            let effective_from = match body.apply_from {
                Some(ApplyFrom::NextRotation) => loaded.spec.next_rotation_boundary(Layer::Primary, now).unwrap_or(now),
                _ => now,
            };
            // Versions scheduled after this one are replaced by it.
            sqlx::query("DELETE FROM schedule_versions WHERE schedule_id = $1 AND effective_from > $2").bind(id).bind(effective_from).execute(&mut *tx).await?;
            repo::insert_version(&mut tx, id, effective_from, rotation.interval, rotation.unit, &rotation.primary, &rotation.secondary, Some(ctx.user_id()))
                .await?;
            change = json!({ "rotation": "new_version", "effective_from": effective_from });
        }
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "schedule.updated", "schedule", Some(id), change.clone()).await?;
    if body.status.as_deref() == Some("archived") {
        audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "schedule.archived", "schedule", Some(id), json!({})).await?;
    }
    let payload = EventPayload {
        schedule_id: Some(id),
        schedule_name: Some(name.map(str::to_string).unwrap_or_else(|| row.name.clone())),
        timezone: Some(times.tz.name().into()),
        actor_name: Some(ctx.user.display_name.clone()),
        detail: Some(if body.rotation.is_some() {
            "The rotation changed. Upcoming shifts were recalculated.".into()
        } else {
            "Schedule details changed.".into()
        }),
        ..Default::default()
    };
    notifications::enqueue(
        &mut tx,
        NewEvent {
            organization_id: ctx.organization_id,
            schedule_id: Some(id),
            team_id: body.team_id.unwrap_or(row.team_id),
            kind: EventKind::ScheduleUpdated,
            tags: vec![],
            dedupe_key: format!("schedule_updated:{id}:{}", Uuid::new_v4()),
            payload: &payload,
            scheduled_for: now,
            exclude_integration_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    planner::plan_schedule_by_id(&state, id).await.ok();
    get(State(state), ctx, Path(id)).await
}

/// Schedules are archived rather than deleted so their history and audit trail stay intact.
pub async fn archive(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageSchedules)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("UPDATE schedules SET status = 'archived', updated_at = now() WHERE id = $1 AND organization_id = $2 AND status <> 'archived'")
        .bind(id)
        .bind(ctx.organization_id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "schedule.archived", "schedule", Some(id), json!({})).await?;
    tx.commit().await?;
    planner::plan_schedule_by_id(&state, id).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct AtQuery {
    at: Option<DateTime<Utc>>,
}

pub async fn on_call(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>, Query(q): Query<AtQuery>) -> AppResult<Json<OnCallNow>> {
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    Ok(Json(on_call_now(&loaded.spec, loaded.row.status == "archived", q.at.unwrap_or_else(Utc::now))))
}

#[derive(Deserialize)]
pub struct RangeQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: Option<usize>,
}

pub async fn assignments(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>, Query(q): Query<RangeQuery>) -> AppResult<Json<Vec<Shift>>> {
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    let from = q.from.unwrap_or_else(|| Utc::now() - Duration::days(7));
    let to = q.to.unwrap_or(from + Duration::days(35));
    if to <= from {
        return Err(AppError::validation("to", "must be after from"));
    }
    if to - from > Duration::days(engine::MAX_WINDOW_DAYS) {
        return Err(AppError::validation("to", format!("the range can be at most {} days", engine::MAX_WINDOW_DAYS)));
    }
    let mut shifts = loaded.spec.shifts(from, to);
    shifts.truncate(q.limit.unwrap_or(1000).min(2000));
    Ok(Json(shifts))
}

pub async fn upcoming(
    State(state): State<AppState>,
    ctx: OrgContext,
    Path(id): Path<Uuid>,
    Query(q): Query<RangeQuery>,
) -> AppResult<Json<Vec<engine::Transition>>> {
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    let from = q.from.unwrap_or_else(Utc::now);
    let mut transitions: Vec<engine::Transition> = Layer::ALL.iter().flat_map(|l| loaded.spec.transitions(*l, from, from + Duration::days(180))).collect();
    transitions.sort_by_key(|t| t.at);
    transitions.truncate(q.limit.unwrap_or(20).min(200));
    Ok(Json(transitions))
}

#[derive(Deserialize)]
pub struct OverrideInput {
    layer: Layer,
    user_id: Uuid,
    /// Local wall-clock times in the schedule's timezone, `YYYY-MM-DDTHH:MM`.
    start_local: String,
    end_local: String,
    #[serde(default)]
    reason: String,
}

pub async fn create_override(
    State(state): State<AppState>,
    ctx: OrgContext,
    Path(id): Path<Uuid>,
    Json(body): Json<OverrideInput>,
) -> AppResult<(StatusCode, Json<OverrideRow>)> {
    ctx.require(Permission::ManageSchedules)?;
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    if loaded.row.status == "archived" {
        return Err(AppError::Conflict("this schedule is archived".into()));
    }
    let tz = loaded.row.tz();
    let start = parse_local("start_local", &body.start_local, tz)?;
    let end = parse_local("end_local", &body.end_local, tz)?;
    if end <= start {
        return Err(AppError::validation("end_local", "the end must be after the start"));
    }
    if end - start > Duration::days(90) {
        return Err(AppError::validation("end_local", "an override can last at most 90 days"));
    }
    if end <= Utc::now() {
        return Err(AppError::validation("end_local", "an override must end in the future"));
    }
    if body.reason.chars().count() > 500 {
        return Err(AppError::validation("reason", "keep the reason under 500 characters"));
    }
    let replaced = loaded.spec.on_call_at(body.layer, start).map(|s| s.user_id);

    let mut tx = state.db.begin().await?;
    ensure_members(&mut tx, ctx.organization_id, &[body.user_id]).await?;
    let row: OverrideRow = sqlx::query_as(
        "INSERT INTO schedule_overrides (schedule_id, layer, user_id, start_at, end_at, reason, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, layer, user_id, start_at, end_at, reason, created_at, created_by, canceled_at",
    )
    .bind(id)
    .bind(body.layer.as_str())
    .bind(body.user_id)
    .bind(start)
    .bind(end)
    .bind(body.reason.trim())
    .bind(ctx.user_id())
    .fetch_one(&mut *tx)
    .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "override.created",
        "schedule",
        Some(id),
        json!({ "override_id": row.id, "user_id": body.user_id, "layer": body.layer.as_str() }),
    )
    .await?;
    let people = planner::load_people(&mut tx, &[Some(body.user_id), replaced].into_iter().flatten().collect::<Vec<_>>()).await?;
    let person = people.get(&body.user_id).cloned();
    let payload = EventPayload {
        schedule_id: Some(id),
        schedule_name: Some(loaded.row.name.clone()),
        timezone: Some(loaded.row.timezone.clone()),
        at: Some(start),
        shift_end: Some(end),
        layer: Some(body.layer.as_str().into()),
        primary: if body.layer == Layer::Primary { person.clone() } else { None },
        secondary: if body.layer == Layer::Secondary { person } else { None },
        previous_primary: if body.layer == Layer::Primary { replaced.and_then(|r| people.get(&r).cloned()) } else { None },
        previous_secondary: if body.layer == Layer::Secondary { replaced.and_then(|r| people.get(&r).cloned()) } else { None },
        reason: Some(body.reason.trim().to_string()).filter(|r| !r.is_empty()),
        actor_name: Some(ctx.user.display_name.clone()),
        ..Default::default()
    };
    notifications::enqueue(
        &mut tx,
        NewEvent {
            organization_id: ctx.organization_id,
            schedule_id: Some(id),
            team_id: loaded.row.team_id,
            kind: EventKind::OverrideCreated,
            tags: vec![],
            dedupe_key: format!("override_created:{}", row.id),
            payload: &payload,
            scheduled_for: Utc::now(),
            exclude_integration_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    planner::plan_schedule_by_id(&state, id).await.ok();
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn cancel_override(State(state): State<AppState>, ctx: OrgContext, Path((id, override_id)): Path<(Uuid, Uuid)>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageSchedules)?;
    let loaded = repo::load_one(&state.db, ctx.organization_id, id).await?;
    let mut tx = state.db.begin().await?;
    let row: Option<OverrideRow> = sqlx::query_as(
        "UPDATE schedule_overrides SET canceled_at = now(), canceled_by = $3 WHERE id = $1 AND schedule_id = $2 AND canceled_at IS NULL
         RETURNING id, layer, user_id, start_at, end_at, reason, created_at, created_by, canceled_at",
    )
    .bind(override_id)
    .bind(id)
    .bind(ctx.user_id())
    .fetch_optional(&mut *tx)
    .await?;
    let row = row.ok_or(AppError::NotFound)?;
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "override.canceled", "schedule", Some(id), json!({ "override_id": override_id }))
        .await?;
    let people = planner::load_people(&mut tx, &[row.user_id]).await?;
    let person = people.get(&row.user_id).cloned();
    let layer = Layer::parse(&row.layer).unwrap_or(Layer::Primary);
    let payload = EventPayload {
        schedule_id: Some(id),
        schedule_name: Some(loaded.row.name.clone()),
        timezone: Some(loaded.row.timezone.clone()),
        at: Some(row.start_at),
        shift_end: Some(row.end_at),
        layer: Some(row.layer.clone()),
        primary: if layer == Layer::Primary { person.clone() } else { None },
        secondary: if layer == Layer::Secondary { person } else { None },
        actor_name: Some(ctx.user.display_name.clone()),
        ..Default::default()
    };
    notifications::enqueue(
        &mut tx,
        NewEvent {
            organization_id: ctx.organization_id,
            schedule_id: Some(id),
            team_id: loaded.row.team_id,
            kind: EventKind::OverrideCanceled,
            tags: vec![],
            dedupe_key: format!("override_canceled:{override_id}"),
            payload: &payload,
            scheduled_for: Utc::now(),
            exclude_integration_id: None,
        },
    )
    .await?;
    tx.commit().await?;
    planner::plan_schedule_by_id(&state, id).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

// ---------- organization overview ----------

#[derive(Serialize)]
pub struct OverviewSchedule {
    pub id: Uuid,
    pub name: String,
    pub team_id: Option<Uuid>,
    pub timezone: String,
    pub state: &'static str,
    pub on_call: OnCallNow,
}

#[derive(Serialize)]
pub struct UpcomingHandover {
    pub schedule_id: Uuid,
    pub schedule_name: String,
    pub timezone: String,
    pub team_id: Option<Uuid>,
    #[serde(flatten)]
    pub transition: engine::Transition,
}

#[derive(Serialize)]
pub struct Overview {
    pub schedules: Vec<OverviewSchedule>,
    pub upcoming: Vec<UpcomingHandover>,
}

/// Current on-call across the organization plus the next hand-overs, for the dashboard.
pub async fn overview(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Overview>> {
    let loaded = repo::load_many(&state.db, ctx.organization_id, "AND status <> 'archived'").await?;
    let now = Utc::now();
    let mut upcoming = Vec::new();
    let mut schedules = Vec::new();
    for l in &loaded {
        let v = view(l, now);
        for layer in Layer::ALL {
            for t in l.spec.transitions(layer, now, now + Duration::days(30)) {
                upcoming.push(UpcomingHandover {
                    schedule_id: l.row.id,
                    schedule_name: l.row.name.clone(),
                    timezone: l.row.timezone.clone(),
                    team_id: l.row.team_id,
                    transition: t,
                });
            }
        }
        schedules.push(OverviewSchedule {
            id: l.row.id,
            name: l.row.name.clone(),
            team_id: l.row.team_id,
            timezone: l.row.timezone.clone(),
            state: v.state,
            on_call: v.on_call,
        });
    }
    upcoming.sort_by_key(|u| u.transition.at);
    upcoming.truncate(25);
    Ok(Json(Overview { schedules, upcoming }))
}
