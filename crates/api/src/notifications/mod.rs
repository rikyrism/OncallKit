//! Notification events, planning and delivery.
//!
//! Flow: something happens (an API action, or the planner finding an upcoming rotation) →
//! a row in `notification_events` with a deterministic `dedupe_key` → when due, the dispatcher
//! fans it out to matching integrations as rows in `notification_deliveries` → delivery workers
//! claim those rows with `FOR UPDATE SKIP LOCKED` and a lease, send them, and retry with
//! backoff. Every step is idempotent, so any number of workers can run and restart safely.

pub mod dispatcher;
pub mod message;
pub mod planner;
pub mod routes;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Event kinds integrations can subscribe to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ScheduleCreated,
    ScheduleUpdated,
    RotationApproaching,
    RotationChanged,
    PrimaryChanged,
    SecondaryChanged,
    OverrideCreated,
    OverrideCanceled,
    DeliveryFailed,
    Test,
}

impl EventKind {
    pub const SUBSCRIBABLE: [EventKind; 9] = [
        EventKind::ScheduleCreated,
        EventKind::ScheduleUpdated,
        EventKind::RotationApproaching,
        EventKind::RotationChanged,
        EventKind::PrimaryChanged,
        EventKind::SecondaryChanged,
        EventKind::OverrideCreated,
        EventKind::OverrideCanceled,
        EventKind::DeliveryFailed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::ScheduleCreated => "schedule_created",
            EventKind::ScheduleUpdated => "schedule_updated",
            EventKind::RotationApproaching => "rotation_approaching",
            EventKind::RotationChanged => "rotation_changed",
            EventKind::PrimaryChanged => "primary_changed",
            EventKind::SecondaryChanged => "secondary_changed",
            EventKind::OverrideCreated => "override_created",
            EventKind::OverrideCanceled => "override_canceled",
            EventKind::DeliveryFailed => "delivery_failed",
            EventKind::Test => "test",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::SUBSCRIBABLE.into_iter().chain([EventKind::Test]).find(|k| k.as_str() == value)
    }

    /// Kinds produced by the planner from the schedule itself. Pending ones are re-evaluated
    /// (and superseded) whenever the schedule changes.
    pub fn is_planned(self) -> bool {
        matches!(self, EventKind::RotationApproaching | EventKind::RotationChanged | EventKind::PrimaryChanged | EventKind::SecondaryChanged)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonRef {
    pub user_id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_user_id: Option<String>,
}

/// Everything needed to render a message, captured when the event is created.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventPayload {
    pub schedule_id: Option<Uuid>,
    pub schedule_name: Option<String>,
    pub team_name: Option<String>,
    pub timezone: Option<String>,
    pub at: Option<DateTime<Utc>>,
    pub primary: Option<PersonRef>,
    pub secondary: Option<PersonRef>,
    pub previous_primary: Option<PersonRef>,
    pub previous_secondary: Option<PersonRef>,
    pub next_rotation_at: Option<DateTime<Utc>>,
    pub shift_end: Option<DateTime<Utc>>,
    pub layer: Option<String>,
    pub offset_minutes: Option<i32>,
    pub reason: Option<String>,
    pub actor_name: Option<String>,
    pub detail: Option<String>,
    pub cause: Option<String>,
}

pub struct NewEvent<'a> {
    pub organization_id: Uuid,
    pub schedule_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub kind: EventKind,
    pub tags: Vec<EventKind>,
    pub dedupe_key: String,
    pub payload: &'a EventPayload,
    pub scheduled_for: DateTime<Utc>,
    pub exclude_integration_id: Option<Uuid>,
}

/// Insert an event unless one with the same dedupe key exists. A superseded event that becomes
/// relevant again (the edit was reverted) is revived instead of duplicated.
pub async fn enqueue(tx: &mut Transaction<'_, Postgres>, event: NewEvent<'_>) -> sqlx::Result<bool> {
    let mut tags: Vec<&str> = event.tags.iter().map(|t| t.as_str()).collect();
    if !tags.contains(&event.kind.as_str()) {
        tags.insert(0, event.kind.as_str());
    }
    let result = sqlx::query(
        "INSERT INTO notification_events (organization_id, schedule_id, team_id, event_type, tags, dedupe_key, payload, scheduled_for, exclude_integration_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (dedupe_key) DO UPDATE SET status = 'pending', payload = EXCLUDED.payload, scheduled_for = EXCLUDED.scheduled_for
         WHERE notification_events.status = 'superseded'",
    )
    .bind(event.organization_id)
    .bind(event.schedule_id)
    .bind(event.team_id)
    .bind(event.kind.as_str())
    .bind(&tags)
    .bind(&event.dedupe_key)
    .bind(sqlx::types::Json(event.payload))
    .bind(event.scheduled_for)
    .bind(event.exclude_integration_id)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for kind in EventKind::SUBSCRIBABLE {
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EventKind::parse("nope"), None);
    }
}
