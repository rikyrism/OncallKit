use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{MatchedPath, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use prometheus::{Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder};
use serde_json::json;

use crate::state::AppState;

#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,
    pub http_requests: IntCounterVec,
    pub http_duration: HistogramVec,
    pub jobs_processed: IntCounterVec,
    pub deliveries: IntCounterVec,
    pub delivery_retries: IntCounterVec,
    pub pending_deliveries: IntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new_custom(Some("oncallkit".into()), None).expect("valid registry");
        let http_requests = IntCounterVec::new(Opts::new("http_requests_total", "HTTP requests"), &["method", "route", "status"]).unwrap();
        let http_duration = HistogramVec::new(
            HistogramOpts::new("http_request_duration_seconds", "HTTP request latency").buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "route"],
        )
        .unwrap();
        let jobs_processed = IntCounterVec::new(Opts::new("jobs_processed_total", "Background job runs"), &["job", "outcome"]).unwrap();
        let deliveries = IntCounterVec::new(Opts::new("notification_deliveries_total", "Notification delivery outcomes"), &["kind", "outcome"]).unwrap();
        let delivery_retries = IntCounterVec::new(Opts::new("notification_retries_total", "Notification deliveries scheduled for retry"), &["kind"]).unwrap();
        let pending_deliveries = IntGauge::new("notification_deliveries_pending", "Deliveries waiting to be sent").unwrap();
        for c in [
            Box::new(http_requests.clone()) as Box<dyn prometheus::core::Collector>,
            Box::new(http_duration.clone()),
            Box::new(jobs_processed.clone()),
            Box::new(deliveries.clone()),
            Box::new(delivery_retries.clone()),
            Box::new(pending_deliveries.clone()),
        ] {
            registry.register(c).expect("metric registered once");
        }
        Metrics { registry, http_requests, http_duration, jobs_processed, deliveries, delivery_retries, pending_deliveries }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn track_http(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let route = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_owned()).unwrap_or_else(|| "unmatched".into());
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16().to_string();
    state.metrics.http_requests.with_label_values(&[&method, &route, &status]).inc();
    state.metrics.http_duration.with_label_values(&[&method, &route]).observe(started.elapsed().as_secs_f64());
    response
}

pub async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = &state.config.metrics_token {
        let expected = format!("Bearer {token}");
        let given = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
        if !constant_time_eq(given.as_bytes(), expected.as_bytes()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let mut buf = Vec::new();
    if TextEncoder::new().encode(&state.metrics.registry.gather(), &mut buf).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], buf).into_response()
}

pub async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn readyz(State(state): State<AppState>) -> Response {
    match tokio::time::timeout(Duration::from_secs(2), sqlx::query("SELECT 1").execute(&state.db)).await {
        Ok(Ok(_)) => Json(json!({ "status": "ready", "database": "ok" })).into_response(),
        _ => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "not_ready", "database": "unavailable" }))).into_response(),
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A small fixed-window rate limiter keyed by client address. It is per process; deployments
/// with several API replicas should also rate-limit at the reverse proxy.
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<(IpAddr, &'static str), (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        RateLimiter { limit, window, buckets: Mutex::new(HashMap::new()) }
    }

    pub fn check(&self, ip: IpAddr, scope: &'static str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().expect("rate limiter lock");
        if buckets.len() > 50_000 {
            buckets.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let entry = buckets.entry((ip, scope)).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_blocks_after_limit() {
        let rl = RateLimiter::new(2, Duration::from_secs(60));
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(rl.check(ip, "auth"));
        assert!(rl.check(ip, "auth"));
        assert!(!rl.check(ip, "auth"));
        assert!(rl.check(ip, "other"));
        assert!(rl.check("10.0.0.2".parse().unwrap(), "auth"));
    }

    #[test]
    fn constant_time_eq_compares() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
