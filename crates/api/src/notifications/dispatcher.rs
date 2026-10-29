//! Dispatch (event → deliveries) and delivery (deliveries → Discord), with leases and retries.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use uuid::Uuid;

use super::{enqueue, message, EventKind, EventPayload, NewEvent};
use crate::{integrations::discord::Outcome, state::AppState};

/// How long a claimed row stays reserved for one worker. Longer than any single HTTP attempt,
/// so a live worker always finishes first; a crashed worker's rows become claimable again.
pub const LEASE_SECONDS: i64 = 60;

/// Exponential backoff with jitter: 30s, 1m, 2m, 4m … capped at 1 hour, each scaled by a random
/// factor in [0.5, 1.0] so retries from many deliveries do not synchronise.
pub fn backoff(attempt: i32, jitter: f64) -> StdDuration {
    let exp = 30u64.saturating_mul(1u64 << (attempt.clamp(1, 12) - 1) as u32).min(3600);
    StdDuration::from_secs_f64(exp as f64 * (0.5 + 0.5 * jitter.clamp(0.0, 1.0)))
}

#[derive(sqlx::FromRow)]
struct ClaimedEvent {
    id: Uuid,
    organization_id: Uuid,
    team_id: Option<Uuid>,
    event_type: String,
    tags: Vec<String>,
    payload: sqlx::types::Json<EventPayload>,
    exclude_integration_id: Option<Uuid>,
}

/// Fan due events out to the integrations subscribed to them. Returns how many events were
/// processed.
pub async fn dispatch_due_events(state: &AppState) -> anyhow::Result<usize> {
    let events: Vec<ClaimedEvent> = sqlx::query_as(
        "UPDATE notification_events SET locked_until = now() + make_interval(secs => $1)
         WHERE id IN (
            SELECT id FROM notification_events
            WHERE status = 'pending' AND scheduled_for <= now() AND (locked_until IS NULL OR locked_until < now())
            ORDER BY scheduled_for LIMIT 100 FOR UPDATE SKIP LOCKED)
         RETURNING id, organization_id, team_id, event_type, tags, payload, exclude_integration_id",
    )
    .bind(LEASE_SECONDS as f64)
    .fetch_all(&state.db)
    .await?;

    for event in &events {
        let mut tx = state.db.begin().await?;
        let stale_reminder = event.event_type == EventKind::RotationApproaching.as_str() && event.payload.at.is_some_and(|at| at <= Utc::now());
        let status = if stale_reminder {
            "skipped"
        } else {
            sqlx::query(
                "INSERT INTO notification_deliveries (event_id, integration_id, organization_id)
                 SELECT $1, i.id, i.organization_id FROM integrations i
                 WHERE i.organization_id = $2 AND i.enabled AND i.events && $3
                   AND (i.team_id IS NULL OR i.team_id = $4)
                   AND ($5::uuid IS NULL OR i.id <> $5)
                 ON CONFLICT (event_id, integration_id) DO NOTHING",
            )
            .bind(event.id)
            .bind(event.organization_id)
            .bind(&event.tags)
            .bind(event.team_id)
            .bind(event.exclude_integration_id)
            .execute(&mut *tx)
            .await?;
            "dispatched"
        };
        sqlx::query("UPDATE notification_events SET status = $2, dispatched_at = now(), locked_until = NULL WHERE id = $1 AND status = 'pending'")
            .bind(event.id)
            .bind(status)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        state.metrics.jobs_processed.with_label_values(&["dispatch_event", status]).inc();
    }
    Ok(events.len())
}

#[derive(sqlx::FromRow)]
struct ClaimedDelivery {
    id: Uuid,
    integration_id: Uuid,
    organization_id: Uuid,
    attempts: i32,
}

#[derive(sqlx::FromRow)]
struct DeliveryContext {
    event_type: String,
    payload: sqlx::types::Json<EventPayload>,
    name: String,
    enabled: bool,
    mention_users: bool,
    message_prefix: String,
    max_attempts: i32,
    secret_ciphertext: Vec<u8>,
    secret_nonce: Option<Vec<u8>>,
    team_id: Option<Uuid>,
}

/// Claim and send due deliveries. Returns how many were attempted.
pub async fn deliver_due(state: &AppState) -> anyhow::Result<usize> {
    let claimed: Vec<ClaimedDelivery> = sqlx::query_as(
        "UPDATE notification_deliveries SET status = 'sending', attempts = attempts + 1,
                locked_until = now() + make_interval(secs => $1), updated_at = now()
         WHERE id IN (
            SELECT id FROM notification_deliveries
            WHERE (status = 'pending' AND next_attempt_at <= now()) OR (status = 'sending' AND locked_until < now())
            ORDER BY next_attempt_at LIMIT 25 FOR UPDATE SKIP LOCKED)
         RETURNING id, integration_id, organization_id, attempts",
    )
    .bind(LEASE_SECONDS as f64)
    .fetch_all(&state.db)
    .await?;

    let mut handles = Vec::new();
    for delivery in claimed {
        let state = state.clone();
        handles.push(tokio::spawn(async move {
            if let Err(err) = deliver_one(&state, &delivery).await {
                tracing::error!(delivery_id = %delivery.id, error = %format!("{err:#}"), "delivery bookkeeping failed");
            }
        }));
    }
    let count = handles.len();
    for h in handles {
        let _ = h.await;
    }
    if let Ok(pending) =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notification_deliveries WHERE status IN ('pending', 'sending')").fetch_one(&state.db).await
    {
        state.metrics.pending_deliveries.set(pending);
    }
    Ok(count)
}

async fn deliver_one(state: &AppState, delivery: &ClaimedDelivery) -> anyhow::Result<()> {
    let ctx: DeliveryContext = sqlx::query_as(
        "SELECT e.event_type, e.payload, i.name, i.enabled, i.mention_users, i.message_prefix, i.max_attempts,
                i.secret_ciphertext, i.secret_nonce, i.team_id
         FROM notification_deliveries d
         JOIN notification_events e ON e.id = d.event_id
         JOIN integrations i ON i.id = d.integration_id
         WHERE d.id = $1",
    )
    .bind(delivery.id)
    .fetch_one(&state.db)
    .await?;

    if !ctx.enabled {
        finish(state, delivery, "canceled", None, Some("integration disabled".into()), None, false).await?;
        return Ok(());
    }
    let kind = EventKind::parse(&ctx.event_type).unwrap_or(EventKind::Test);
    let started = Utc::now();
    let outcome = match state.secrets.open(&ctx.secret_ciphertext, ctx.secret_nonce.as_deref()) {
        Ok(webhook_url) => {
            let msg = message::render(
                kind,
                &ctx.payload,
                &message::RenderOptions { app_url: state.config.public_url.as_str(), mention_users: ctx.mention_users, prefix: &ctx.message_prefix },
            );
            state.discord.send(&webhook_url, &msg).await
        }
        Err(err) => Outcome::Failed { status: None, error: format!("{err}") },
    };
    record_attempt(state, delivery.id, delivery.attempts, started, &outcome).await?;

    match outcome {
        Outcome::Delivered { status } => {
            finish(state, delivery, "succeeded", Some(status), None, None, true).await?;
            state.metrics.deliveries.with_label_values(&["discord", "succeeded"]).inc();
        }
        Outcome::Retry { status, error, retry_after } if delivery.attempts < ctx.max_attempts => {
            let wait = retry_after.unwrap_or_else(|| backoff(delivery.attempts, rand::thread_rng().gen()));
            let next = Utc::now() + Duration::from_std(wait).unwrap_or(Duration::minutes(1));
            finish(state, delivery, "pending", status, Some(error), Some(next), false).await?;
            state.metrics.delivery_retries.with_label_values(&["discord"]).inc();
        }
        Outcome::Retry { status, error, .. } | Outcome::Failed { status, error } => {
            finish(state, delivery, "failed", status, Some(error.clone()), None, false).await?;
            state.metrics.deliveries.with_label_values(&["discord", "failed"]).inc();
            if kind != EventKind::DeliveryFailed && kind != EventKind::Test {
                report_failure(state, delivery, &ctx, &error).await?;
            }
        }
    }
    Ok(())
}

async fn record_attempt(state: &AppState, delivery_id: Uuid, attempt: i32, started: DateTime<Utc>, outcome: &Outcome) -> sqlx::Result<()> {
    let (status, success, error) = match outcome {
        Outcome::Delivered { status } => (Some(*status), true, None),
        Outcome::Retry { status, error, .. } | Outcome::Failed { status, error } => (*status, false, Some(error.clone())),
    };
    sqlx::query(
        "INSERT INTO notification_delivery_attempts (delivery_id, attempt, started_at, finished_at, status_code, success, error)
         VALUES ($1, $2, $3, now(), $4, $5, $6)",
    )
    .bind(delivery_id)
    .bind(attempt)
    .bind(started)
    .bind(status.map(i32::from))
    .bind(success)
    .bind(error)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn finish(
    state: &AppState,
    delivery: &ClaimedDelivery,
    status: &str,
    status_code: Option<u16>,
    error: Option<String>,
    next_attempt_at: Option<DateTime<Utc>>,
    delivered: bool,
) -> sqlx::Result<()> {
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE notification_deliveries SET status = $2, last_status_code = $3, last_error = $4,
                next_attempt_at = COALESCE($5, next_attempt_at), locked_until = NULL,
                delivered_at = CASE WHEN $6 THEN now() ELSE delivered_at END, updated_at = now()
         WHERE id = $1",
    )
    .bind(delivery.id)
    .bind(status)
    .bind(status_code.map(i32::from))
    .bind(&error)
    .bind(next_attempt_at)
    .bind(delivered)
    .execute(&mut *tx)
    .await?;
    if status != "pending" {
        sqlx::query("UPDATE integrations SET last_delivery_status = $2, last_delivery_at = now() WHERE id = $1")
            .bind(delivery.integration_id)
            .bind(status)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

async fn report_failure(state: &AppState, delivery: &ClaimedDelivery, ctx: &DeliveryContext, error: &str) -> anyhow::Result<()> {
    let mut tx = state.db.begin().await?;
    let payload = EventPayload {
        schedule_id: ctx.payload.schedule_id,
        schedule_name: ctx.payload.schedule_name.clone(),
        timezone: ctx.payload.timezone.clone(),
        detail: Some(format!(
            "Integration \"{}\" failed to deliver a {} notification after {} attempts. Last error: {}",
            ctx.name,
            ctx.event_type.replace('_', " "),
            delivery.attempts,
            error
        )),
        ..Default::default()
    };
    enqueue(
        &mut tx,
        NewEvent {
            organization_id: delivery.organization_id,
            schedule_id: ctx.payload.schedule_id,
            team_id: ctx.team_id,
            kind: EventKind::DeliveryFailed,
            tags: vec![],
            dedupe_key: format!("delivery_failed:{}", delivery.id),
            payload: &payload,
            scheduled_for: Utc::now(),
            exclude_integration_id: Some(delivery.integration_id),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::backoff;
    use std::time::Duration;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(1, 1.0), Duration::from_secs(30));
        assert_eq!(backoff(2, 1.0), Duration::from_secs(60));
        assert_eq!(backoff(4, 1.0), Duration::from_secs(240));
        assert_eq!(backoff(12, 1.0), Duration::from_secs(3600));
        assert_eq!(backoff(1, 0.0), Duration::from_secs(15));
        assert!(backoff(30, 1.0) <= Duration::from_secs(3600));
    }
}
