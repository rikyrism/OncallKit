//! The background worker loop: plan → dispatch → deliver, plus housekeeping.
//! Every step works off persisted state, so stopping and starting the worker at any moment
//! loses nothing.

use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::{
    auth,
    notifications::{dispatcher, planner},
    state::AppState,
};

const PLAN_EVERY: Duration = Duration::from_secs(60);
const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(3600);

/// Run one full cycle. Exposed for tests and for `oncallkit worker --once`.
pub async fn tick(state: &AppState, plan: bool) -> anyhow::Result<()> {
    if plan {
        let created = planner::plan_all(state).await?;
        state.metrics.jobs_processed.with_label_values(&["plan", "ok"]).inc();
        if created > 0 {
            tracing::info!(created, "planned notification events");
        }
    }
    // Drain: keep going while there is work so bursts clear quickly.
    for _ in 0..20 {
        let events = dispatcher::dispatch_due_events(state).await?;
        let deliveries = dispatcher::deliver_due(state).await?;
        if events == 0 && deliveries == 0 {
            break;
        }
    }
    Ok(())
}

pub async fn run(state: AppState, mut shutdown: watch::Receiver<bool>) {
    tracing::info!("worker started");
    let mut last_plan: Option<Instant> = None;
    let mut last_housekeeping: Option<Instant> = None;
    loop {
        let plan = last_plan.is_none_or(|t| t.elapsed() >= PLAN_EVERY);
        if let Err(err) = tick(&state, plan).await {
            state.metrics.jobs_processed.with_label_values(&["tick", "error"]).inc();
            tracing::error!(error = %format!("{err:#}"), "worker tick failed");
        } else if plan {
            last_plan = Some(Instant::now());
        }
        if last_housekeeping.is_none_or(|t| t.elapsed() >= HOUSEKEEPING_EVERY) {
            if let Err(err) = auth::routes::purge_expired(&state.db).await {
                tracing::warn!(error = %err, "housekeeping failed");
            }
            last_housekeeping = Some(Instant::now());
        }
        tokio::select! {
            _ = tokio::time::sleep(state.config.worker_poll_interval) => {}
            _ = shutdown.changed() => break,
        }
    }
    tracing::info!("worker stopped");
}
