//! Loading and saving schedules. Turns database rows into an engine [`ScheduleSpec`].

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use serde_json::json;
use sqlx::{PgExecutor, Postgres, Transaction};
use uuid::Uuid;

use super::engine::{Layer, Override, RotationUnit, RotationVersion, ScheduleSpec};
use crate::{audit, error::AppResult};

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct ScheduleRow {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub team_id: Option<Uuid>,
    pub name: String,
    pub description: String,
    pub timezone: String,
    pub start_at: DateTime<Utc>,
    pub end_at: Option<DateTime<Utc>>,
    pub status: String,
    pub reminder_offsets_minutes: Vec<i32>,
    pub notify_on_boundary: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ScheduleRow {
    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(chrono_tz::UTC)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VersionView {
    pub id: Uuid,
    pub effective_from: DateTime<Utc>,
    pub interval: u32,
    pub unit: RotationUnit,
    pub primary: Vec<Uuid>,
    pub secondary: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct OverrideRow {
    pub id: Uuid,
    pub layer: String,
    pub user_id: Uuid,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
    pub canceled_at: Option<DateTime<Utc>>,
}

pub struct LoadedSchedule {
    pub row: ScheduleRow,
    pub versions: Vec<VersionView>,
    pub overrides: Vec<OverrideRow>,
    pub spec: ScheduleSpec,
}

pub const SCHEDULE_COLUMNS: &str =
    "id, organization_id, team_id, name, description, timezone, start_at, end_at, status, reminder_offsets_minutes, notify_on_boundary, created_at, updated_at";

pub async fn load_versions(conn: &mut sqlx::PgConnection, schedule_ids: &[Uuid]) -> sqlx::Result<HashMap<Uuid, Vec<VersionView>>> {
    let versions: Vec<(Uuid, Uuid, DateTime<Utc>, i32, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, schedule_id, effective_from, rotation_interval, rotation_unit, created_at FROM schedule_versions
         WHERE schedule_id = ANY($1) ORDER BY effective_from",
    )
    .bind(schedule_ids)
    .fetch_all(&mut *conn)
    .await?;
    let version_ids: Vec<Uuid> = versions.iter().map(|v| v.0).collect();
    let participants: Vec<(Uuid, String, Uuid)> =
        sqlx::query_as("SELECT version_id, layer, user_id FROM schedule_rotation_participants WHERE version_id = ANY($1) ORDER BY version_id, layer, position")
            .bind(&version_ids)
            .fetch_all(&mut *conn)
            .await?;
    let mut by_version: HashMap<Uuid, (Vec<Uuid>, Vec<Uuid>)> = HashMap::new();
    for (version_id, layer, user_id) in participants {
        let entry = by_version.entry(version_id).or_default();
        match Layer::parse(&layer) {
            Some(Layer::Primary) => entry.0.push(user_id),
            Some(Layer::Secondary) => entry.1.push(user_id),
            None => {}
        }
    }
    let mut out: HashMap<Uuid, Vec<VersionView>> = HashMap::new();
    for (id, schedule_id, effective_from, interval, unit, created_at) in versions {
        let (primary, secondary) = by_version.remove(&id).unwrap_or_default();
        out.entry(schedule_id).or_default().push(VersionView {
            id,
            effective_from,
            interval: interval.max(1) as u32,
            unit: RotationUnit::parse(&unit).unwrap_or(RotationUnit::Days),
            primary,
            secondary,
            created_at,
        });
    }
    Ok(out)
}

pub async fn load_overrides<'e>(db: impl PgExecutor<'e>, schedule_ids: &[Uuid]) -> sqlx::Result<HashMap<Uuid, Vec<OverrideRow>>> {
    let rows: Vec<(Uuid, OverrideRow)> = sqlx::query_as::<_, OverrideWithSchedule>(
        "SELECT schedule_id, id, layer, user_id, start_at, end_at, reason, created_at, created_by, canceled_at FROM schedule_overrides
         WHERE schedule_id = ANY($1) ORDER BY created_at",
    )
    .bind(schedule_ids)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|r| (r.schedule_id, r.row))
    .collect();
    let mut out: HashMap<Uuid, Vec<OverrideRow>> = HashMap::new();
    for (schedule_id, row) in rows {
        out.entry(schedule_id).or_default().push(row);
    }
    Ok(out)
}

#[derive(sqlx::FromRow)]
struct OverrideWithSchedule {
    schedule_id: Uuid,
    #[sqlx(flatten)]
    row: OverrideRow,
}

pub fn build_spec(row: &ScheduleRow, versions: &[VersionView], overrides: &[OverrideRow]) -> ScheduleSpec {
    ScheduleSpec::new(
        row.tz(),
        row.start_at,
        row.end_at,
        versions
            .iter()
            .map(|v| RotationVersion {
                effective_from: v.effective_from,
                interval: v.interval,
                unit: v.unit,
                primary: v.primary.clone(),
                secondary: v.secondary.clone(),
            })
            .collect(),
        overrides
            .iter()
            .filter(|o| o.canceled_at.is_none())
            .filter_map(|o| Layer::parse(&o.layer).map(|layer| Override { id: o.id, layer, user_id: o.user_id, start: o.start_at, end: o.end_at }))
            .collect(),
    )
}

/// Load every schedule matching `filter_sql` (which may reference `$1` = organization id).
pub async fn load_many(db: &sqlx::PgPool, organization_id: Uuid, extra_where: &str) -> sqlx::Result<Vec<LoadedSchedule>> {
    let rows: Vec<ScheduleRow> =
        sqlx::query_as(&format!("SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE organization_id = $1 {extra_where} ORDER BY lower(name)"))
            .bind(organization_id)
            .fetch_all(db)
            .await?;
    assemble(db, rows).await
}

pub async fn assemble(db: &sqlx::PgPool, rows: Vec<ScheduleRow>) -> sqlx::Result<Vec<LoadedSchedule>> {
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut conn = db.acquire().await?;
    let mut versions = load_versions(&mut conn, &ids).await?;
    let mut overrides = load_overrides(&mut *conn, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let versions = versions.remove(&row.id).unwrap_or_default();
            let overrides = overrides.remove(&row.id).unwrap_or_default();
            let spec = build_spec(&row, &versions, &overrides);
            LoadedSchedule { row, versions, overrides, spec }
        })
        .collect())
}

pub async fn load_one(db: &sqlx::PgPool, organization_id: Uuid, schedule_id: Uuid) -> AppResult<LoadedSchedule> {
    let row: ScheduleRow = sqlx::query_as(&format!("SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE id = $1 AND organization_id = $2"))
        .bind(schedule_id)
        .bind(organization_id)
        .fetch_optional(db)
        .await?
        .ok_or(crate::error::AppError::NotFound)?;
    Ok(assemble(db, vec![row]).await?.pop().expect("one schedule"))
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_version(
    tx: &mut Transaction<'_, Postgres>,
    schedule_id: Uuid,
    effective_from: DateTime<Utc>,
    interval: u32,
    unit: RotationUnit,
    primary: &[Uuid],
    secondary: &[Uuid],
    actor: Option<Uuid>,
) -> sqlx::Result<Uuid> {
    let version_id: Uuid = sqlx::query_scalar(
        "INSERT INTO schedule_versions (schedule_id, effective_from, rotation_interval, rotation_unit, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (schedule_id, effective_from) DO UPDATE SET rotation_interval = EXCLUDED.rotation_interval,
             rotation_unit = EXCLUDED.rotation_unit, created_by = EXCLUDED.created_by, created_at = now()
         RETURNING id",
    )
    .bind(schedule_id)
    .bind(effective_from)
    .bind(interval as i32)
    .bind(unit.as_str())
    .bind(actor)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM schedule_rotation_participants WHERE version_id = $1").bind(version_id).execute(&mut **tx).await?;
    for (layer, list) in [(Layer::Primary, primary), (Layer::Secondary, secondary)] {
        for (position, user_id) in list.iter().enumerate() {
            sqlx::query("INSERT INTO schedule_rotation_participants (version_id, layer, position, user_id) VALUES ($1, $2, $3, $4)")
                .bind(version_id)
                .bind(layer.as_str())
                .bind(position as i32)
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(version_id)
}

/// Take a user out of every rotation in the organization from now on. Past shifts are kept by
/// starting a new version at `now`; versions that have not started yet are edited in place.
/// Future overrides assigned to the user are cancelled.
pub async fn remove_user_from_rotations(tx: &mut Transaction<'_, Postgres>, organization_id: Uuid, user_id: Uuid, actor: Uuid) -> AppResult<()> {
    let now = Utc::now();
    let schedule_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT v.schedule_id FROM schedule_versions v
         JOIN schedule_rotation_participants p ON p.version_id = v.id
         JOIN schedules s ON s.id = v.schedule_id
         WHERE s.organization_id = $1 AND p.user_id = $2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_all(&mut **tx)
    .await?;

    for schedule_id in schedule_ids {
        let versions = load_versions(tx, &[schedule_id]).await?.remove(&schedule_id).unwrap_or_default();
        let current = versions.iter().rfind(|v| v.effective_from <= now).cloned();
        for v in versions.iter().filter(|v| v.effective_from > now) {
            if v.primary.contains(&user_id) || v.secondary.contains(&user_id) {
                let primary: Vec<Uuid> = v.primary.iter().copied().filter(|u| *u != user_id).collect();
                let secondary: Vec<Uuid> = v.secondary.iter().copied().filter(|u| *u != user_id).collect();
                insert_version(tx, schedule_id, v.effective_from, v.interval, v.unit, &primary, &secondary, Some(actor)).await?;
            }
        }
        if let Some(v) = current {
            if v.primary.contains(&user_id) || v.secondary.contains(&user_id) {
                let primary: Vec<Uuid> = v.primary.iter().copied().filter(|u| *u != user_id).collect();
                let secondary: Vec<Uuid> = v.secondary.iter().copied().filter(|u| *u != user_id).collect();
                insert_version(tx, schedule_id, now, v.interval, v.unit, &primary, &secondary, Some(actor)).await?;
            }
        }
        audit::record_tx(tx, Some(organization_id), Some(actor), "schedule.participant_removed", "schedule", Some(schedule_id), json!({ "user_id": user_id }))
            .await?;
    }
    sqlx::query(
        "UPDATE schedule_overrides SET canceled_at = now(), canceled_by = $3
         WHERE user_id = $2 AND canceled_at IS NULL AND end_at > now()
           AND schedule_id IN (SELECT id FROM schedules WHERE organization_id = $1)",
    )
    .bind(organization_id)
    .bind(user_id)
    .bind(actor)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
