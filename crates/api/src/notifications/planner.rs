//! The planner turns schedule transitions into persisted notification events ahead of time.
//!
//! It is safe to run as often as you like on as many workers as you like: event keys are derived
//! from the transition itself (`handover:{schedule}:{instant}:{who}`), so re-planning never
//! duplicates, and pending events that no longer match the schedule are marked `superseded`.
//! The schedule engine remains the source of truth; the planner only decides what to announce.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::{enqueue, EventKind, EventPayload, NewEvent, PersonRef};
use crate::{
    schedules::{
        engine::{Layer, ShiftSource, Transition},
        repo::{self, LoadedSchedule},
    },
    state::AppState,
};

/// How far back the planner looks for transitions it may have missed while no worker ran.
pub const LOOKBACK: Duration = Duration::hours(6);
/// Reminders that should have gone out more than this long ago are dropped as stale.
pub const REMINDER_GRACE: Duration = Duration::minutes(10);

pub async fn load_people(conn: &mut sqlx::PgConnection, ids: &[Uuid]) -> sqlx::Result<HashMap<Uuid, PersonRef>> {
    let rows: Vec<(Uuid, String, Option<String>)> =
        sqlx::query_as("SELECT id, display_name, discord_user_id FROM users WHERE id = ANY($1)").bind(ids).fetch_all(&mut *conn).await?;
    Ok(rows.into_iter().map(|(user_id, name, discord_user_id)| (user_id, PersonRef { user_id, name, discord_user_id })).collect())
}

pub fn planned_kinds() -> Vec<&'static str> {
    [EventKind::RotationApproaching, EventKind::RotationChanged, EventKind::PrimaryChanged, EventKind::SecondaryChanged].iter().map(|k| k.as_str()).collect()
}

pub fn handover_key(schedule_id: Uuid, at: DateTime<Utc>, primary: Option<Uuid>, secondary: Option<Uuid>) -> String {
    let who = |u: Option<Uuid>| u.map(|u| u.to_string()).unwrap_or_else(|| "none".into());
    format!("handover:{schedule_id}:{}:{}:{}", at.timestamp(), who(primary), who(secondary))
}

pub fn reminder_key(schedule_id: Uuid, layer: Layer, at: DateTime<Utc>, user: Uuid, offset_minutes: i32) -> String {
    format!("reminder:{schedule_id}:{}:{}:{user}:{offset_minutes}", layer.as_str(), at.timestamp())
}

pub struct PlannedEvent {
    pub kind: EventKind,
    pub tags: Vec<EventKind>,
    pub key: String,
    pub scheduled_for: DateTime<Utc>,
    pub payload: EventPayload,
}

/// Pure planning step: which events should exist for this schedule, given the time `now`.
pub fn plan(loaded: &LoadedSchedule, people: &HashMap<Uuid, PersonRef>, team_name: Option<String>, now: DateTime<Utc>) -> Vec<PlannedEvent> {
    let row = &loaded.row;
    if row.status != "active" {
        return vec![];
    }
    let spec = &loaded.spec;
    let max_offset = row.reminder_offsets_minutes.iter().copied().max().unwrap_or(0).max(0);
    let horizon = now + Duration::minutes(i64::from(max_offset)) + Duration::hours(2);
    let lookback = (now - LOOKBACK).max(row.created_at);

    let mut by_instant: BTreeMap<DateTime<Utc>, (Option<Transition>, Option<Transition>)> = BTreeMap::new();
    for layer in Layer::ALL {
        for t in spec.transitions(layer, lookback, horizon) {
            let entry = by_instant.entry(t.at).or_default();
            match layer {
                Layer::Primary => entry.0 = Some(t),
                Layer::Secondary => entry.1 = Some(t),
            }
        }
    }

    let person = |id: Option<Uuid>| id.and_then(|id| people.get(&id).cloned());
    let base = |at: DateTime<Utc>| EventPayload {
        schedule_id: Some(row.id),
        schedule_name: Some(row.name.clone()),
        team_name: team_name.clone(),
        timezone: Some(row.timezone.clone()),
        at: Some(at),
        ..Default::default()
    };

    let mut out = Vec::new();
    for (at, (primary_t, secondary_t)) in &by_instant {
        let at = *at;
        let primary_now = spec.on_call_at(Layer::Primary, at);
        let secondary_now = spec.on_call_at(Layer::Secondary, at);

        if row.notify_on_boundary {
            let by_rotation = [primary_t, secondary_t]
                .iter()
                .any(|t| t.as_ref().and_then(|t| t.shift.as_ref()).is_some_and(|s| matches!(s.source, ShiftSource::Rotation { .. })));
            let mut tags = Vec::new();
            if by_rotation {
                tags.push(EventKind::RotationChanged);
            }
            if primary_t.is_some() {
                tags.push(EventKind::PrimaryChanged);
            }
            if secondary_t.is_some() {
                tags.push(EventKind::SecondaryChanged);
            }
            let kind = tags[0];
            let mut payload = base(at);
            payload.primary = person(primary_now.as_ref().map(|s| s.user_id));
            payload.secondary = person(secondary_now.as_ref().map(|s| s.user_id));
            payload.previous_primary = person(primary_t.as_ref().and_then(|t| t.previous_user_id));
            payload.previous_secondary = person(secondary_t.as_ref().and_then(|t| t.previous_user_id));
            payload.next_rotation_at = spec.next_rotation_boundary(Layer::Primary, at);
            payload.shift_end = primary_now.as_ref().map(|s| s.end);
            payload.cause = Some(if by_rotation { "rotation" } else { "override" }.into());
            out.push(PlannedEvent {
                kind,
                tags,
                key: handover_key(row.id, at, primary_now.as_ref().map(|s| s.user_id), secondary_now.as_ref().map(|s| s.user_id)),
                scheduled_for: at,
                payload,
            });
        }

        for (layer, t) in [(Layer::Primary, primary_t), (Layer::Secondary, secondary_t)] {
            let Some(user_id) = t.as_ref().and_then(|t| t.user_id) else { continue };
            if at <= now {
                continue;
            }
            for offset in &row.reminder_offsets_minutes {
                let fire_at = at - Duration::minutes(i64::from(*offset));
                if fire_at < now - REMINDER_GRACE {
                    continue;
                }
                let mut payload = base(at);
                payload.layer = Some(layer.as_str().into());
                payload.offset_minutes = Some(*offset);
                payload.shift_end = t.as_ref().and_then(|t| t.shift.as_ref()).map(|s| s.end);
                match layer {
                    Layer::Primary => payload.primary = person(Some(user_id)),
                    Layer::Secondary => payload.secondary = person(Some(user_id)),
                }
                out.push(PlannedEvent {
                    kind: EventKind::RotationApproaching,
                    tags: vec![],
                    key: reminder_key(row.id, layer, at, user_id, *offset),
                    scheduled_for: fire_at,
                    payload,
                });
            }
        }
    }
    out
}

pub async fn plan_schedule(state: &AppState, loaded: &LoadedSchedule, now: DateTime<Utc>) -> anyhow::Result<usize> {
    let mut tx = state.db.begin().await?;
    // Serialise planning per schedule across workers.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))").bind(loaded.row.id.to_string()).execute(&mut *tx).await?;

    let mut ids: Vec<Uuid> = loaded.versions.iter().flat_map(|v| v.primary.iter().chain(v.secondary.iter()).copied()).collect();
    ids.extend(loaded.overrides.iter().map(|o| o.user_id));
    ids.sort();
    ids.dedup();
    let people = load_people(&mut tx, &ids).await?;
    let team_name: Option<String> = match loaded.row.team_id {
        Some(team_id) => sqlx::query_scalar("SELECT name FROM teams WHERE id = $1").bind(team_id).fetch_optional(&mut *tx).await?,
        None => None,
    };

    let planned = plan(loaded, &people, team_name, now);
    let mut created = 0;
    for event in &planned {
        let inserted = enqueue(
            &mut tx,
            NewEvent {
                organization_id: loaded.row.organization_id,
                schedule_id: Some(loaded.row.id),
                team_id: loaded.row.team_id,
                kind: event.kind,
                tags: event.tags.clone(),
                dedupe_key: event.key.clone(),
                payload: &event.payload,
                scheduled_for: event.scheduled_for,
                exclude_integration_id: None,
            },
        )
        .await?;
        if inserted {
            created += 1;
        }
    }
    let keys: Vec<&str> = planned.iter().map(|e| e.key.as_str()).collect();
    let superseded = sqlx::query(
        "UPDATE notification_events SET status = 'superseded'
         WHERE schedule_id = $1 AND status = 'pending' AND event_type = ANY($2) AND NOT (dedupe_key = ANY($3))",
    )
    .bind(loaded.row.id)
    .bind(planned_kinds())
    .bind(&keys)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    if created > 0 || superseded > 0 {
        tracing::debug!(schedule_id = %loaded.row.id, created, superseded, "planned notifications");
    }
    Ok(created)
}

pub async fn plan_schedule_by_id(state: &AppState, schedule_id: Uuid) -> anyhow::Result<usize> {
    let row: repo::ScheduleRow =
        sqlx::query_as(&format!("SELECT {} FROM schedules WHERE id = $1", repo::SCHEDULE_COLUMNS)).bind(schedule_id).fetch_one(&state.db).await?;
    let loaded = repo::assemble(&state.db, vec![row]).await?.pop().expect("one schedule");
    plan_schedule(state, &loaded, Utc::now()).await
}

pub async fn plan_all(state: &AppState) -> anyhow::Result<usize> {
    let rows: Vec<repo::ScheduleRow> =
        sqlx::query_as(&format!("SELECT {} FROM schedules WHERE status <> 'archived' OR updated_at > now() - interval '1 day'", repo::SCHEDULE_COLUMNS))
            .fetch_all(&state.db)
            .await?;
    let now = Utc::now();
    let mut created = 0;
    for chunk in rows.chunks(100) {
        for loaded in repo::assemble(&state.db, chunk.to_vec()).await? {
            match plan_schedule(state, &loaded, now).await {
                Ok(n) => created += n,
                Err(err) => tracing::warn!(schedule_id = %loaded.row.id, error = %format!("{err:#}"), "planning failed"),
            }
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedules::{
        engine::{RotationUnit, RotationVersion, ScheduleSpec},
        repo::{ScheduleRow, VersionView},
    };

    fn loaded(start: DateTime<Utc>, offsets: Vec<i32>, users: &[Uuid]) -> LoadedSchedule {
        let row = ScheduleRow {
            id: Uuid::from_u128(1),
            organization_id: Uuid::from_u128(2),
            team_id: None,
            name: "DevOps".into(),
            description: String::new(),
            timezone: "UTC".into(),
            start_at: start,
            end_at: None,
            status: "active".into(),
            reminder_offsets_minutes: offsets,
            notify_on_boundary: true,
            created_at: start - Duration::days(30),
            updated_at: start,
        };
        let version = VersionView {
            id: Uuid::nil(),
            effective_from: start,
            interval: 1,
            unit: RotationUnit::Days,
            primary: users.to_vec(),
            secondary: vec![],
            created_at: start,
        };
        let spec = ScheduleSpec::new(
            chrono_tz::UTC,
            start,
            None,
            vec![RotationVersion { effective_from: start, interval: 1, unit: RotationUnit::Days, primary: users.to_vec(), secondary: vec![] }],
            vec![],
        );
        LoadedSchedule { row, versions: vec![version], overrides: vec![], spec }
    }

    #[test]
    fn plans_one_handover_and_reminders_per_transition() {
        let start: DateTime<Utc> = "2026-11-03T02:00:00Z".parse().unwrap();
        let (a, b) = (Uuid::from_u128(10), Uuid::from_u128(11));
        let l = loaded(start, vec![60], &[a, b]);
        // 30 minutes before the second hand-over.
        let now = start + Duration::days(1) - Duration::minutes(30);
        let events = plan(&l, &HashMap::new(), None, now);
        let handovers: Vec<_> = events.iter().filter(|e| e.kind == EventKind::RotationChanged).collect();
        assert_eq!(handovers.len(), 1);
        assert_eq!(handovers[0].scheduled_for, start + Duration::days(1));
        assert!(handovers[0].tags.contains(&EventKind::PrimaryChanged));
        // The 60-minute reminder was due 30 minutes ago, beyond the grace period: dropped.
        assert!(events.iter().all(|e| e.kind != EventKind::RotationApproaching));
    }

    #[test]
    fn planning_is_deterministic() {
        let start: DateTime<Utc> = "2026-11-03T02:00:00Z".parse().unwrap();
        let l = loaded(start, vec![1440, 60], &[Uuid::from_u128(10), Uuid::from_u128(11)]);
        let now = start + Duration::days(3) + Duration::hours(3);
        let a: Vec<String> = plan(&l, &HashMap::new(), None, now).into_iter().map(|e| e.key).collect();
        let b: Vec<String> = plan(&l, &HashMap::new(), None, now + Duration::seconds(30)).into_iter().map(|e| e.key).collect();
        assert_eq!(a, b);
        assert!(a.iter().any(|k| k.starts_with("reminder:")));
    }

    #[test]
    fn inactive_schedules_plan_nothing() {
        let start: DateTime<Utc> = "2026-11-03T02:00:00Z".parse().unwrap();
        let mut l = loaded(start, vec![60], &[Uuid::from_u128(10)]);
        l.row.status = "paused".into();
        assert!(plan(&l, &HashMap::new(), None, start).is_empty());
    }
}
