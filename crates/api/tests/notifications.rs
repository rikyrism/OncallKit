mod common;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::{webhook_url, TestApp, WEBHOOK_PATH};
use oncallkit::{state::AppState, worker};
use serde_json::{json, Value};
use sqlx::PgPool;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

async fn mock_discord() -> MockServer {
    MockServer::start().await
}

fn app_for(pool: PgPool, server: &MockServer) -> TestApp {
    let uri: url::Url = server.uri().parse().unwrap();
    TestApp::with_config(pool, |c| c.discord_base_override = Some(uri))
}

/// An organization with a schedule whose next hand-over is already due, plus a Discord
/// integration subscribed to `events`.
async fn setup(app: &TestApp, events: &[&str]) -> (common::Session, String, String) {
    let owner = app.login("riky@acme.test").await;
    let people = app.org_with_members(&owner, &[("andi@acme.test", "member")]).await;
    let riky = app.user_id(&owner).await;
    let andi = people[0].1.clone();

    let (status, integration) =
        app.post("/api/v1/integrations/discord", &owner, json!({ "name": "#oncall", "webhook_url": webhook_url(), "events": events })).await;
    assert_eq!(status, StatusCode::CREATED, "{integration}");
    assert!(integration.get("webhook_url").is_none());
    assert!(!integration.to_string().contains("abcdefghijklmnopqrstuvwxyz_0123456789"), "the webhook token must never be returned");

    // Hourly rotation that started 90 minutes ago: a hand-over happened 30 minutes ago.
    let tz: chrono_tz::Tz = "Asia/Jakarta".parse().unwrap();
    let start = (Utc::now() - Duration::minutes(90)).with_timezone(&tz).format("%Y-%m-%dT%H:%M").to_string();
    let (status, schedule) = app
        .post(
            "/api/v1/schedules",
            &owner,
            json!({
                "name": "SRE", "timezone": "Asia/Jakarta", "start_local": start,
                "rotation": { "interval": 1, "unit": "hours", "primary": [riky, andi] },
                "reminder_offsets_minutes": [15]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let id = schedule["id"].as_str().unwrap().to_string();
    // Pretend the schedule was created between its start and that hand-over, so exactly one
    // past boundary is within the planner's reach.
    sqlx::query("UPDATE schedules SET created_at = now() - interval '40 minutes' WHERE id = $1::uuid").bind(&id).execute(&app.state.db).await.unwrap();
    (owner, id, integration["id"].as_str().unwrap().to_string())
}

async fn deliveries(app: &TestApp, s: &common::Session) -> Vec<Value> {
    app.get("/api/v1/notifications/deliveries", s).await.1["items"].as_array().unwrap().clone()
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn test_message_reports_real_result(pool: PgPool) {
    let server = mock_discord().await;
    Mock::given(method("POST"))
        .and(path(WEBHOOK_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "1" })))
        .expect(1)
        .mount(&server)
        .await;
    let app = app_for(pool.clone(), &server);
    let (owner, _, integration) = setup(&app, &["rotation_changed"]).await;

    let (status, result) = app.post(&format!("/api/v1/integrations/{integration}/test"), &owner, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["delivered"], true);

    // A webhook Discord rejects is reported as a failure, never as success.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path(WEBHOOK_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({ "message": "Unknown Webhook" })))
        .mount(&server)
        .await;
    let (_, result) = app.post(&format!("/api/v1/integrations/{integration}/test"), &owner, json!({})).await;
    assert_eq!(result["delivered"], false);
    assert_eq!(result["status_code"], 404);
    let history = deliveries(&app, &owner).await;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["status"], "failed");
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn rejects_non_discord_webhooks(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.login("o@acme.test").await;
    app.post("/api/v1/organizations", &owner, json!({ "name": "Acme" })).await;
    for url in [
        "http://169.254.169.254/latest/meta-data",
        "https://127.0.0.1/api/webhooks/1/abcdefghijklmnopqrstuvwxyz",
        "https://discord.com.attacker.io/api/webhooks/1/abcdefghijklmnopqrstuvwxyz",
    ] {
        let (status, body) = app.post("/api/v1/integrations/discord", &owner, json!({ "name": "x", "webhook_url": url })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{url}: {body}");
    }
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn rotation_boundary_is_delivered_exactly_once(pool: PgPool) {
    let server = mock_discord().await;
    Mock::given(method("POST")).and(path(WEBHOOK_PATH)).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let app = app_for(pool.clone(), &server);
    let (owner, schedule_id, _) = setup(&app, &["rotation_changed"]).await;

    // Several workers racing over the same work, then running again: still one message.
    let (a, b) = tokio::join!(worker::tick(&app.state, true), worker::tick(&app.state, true));
    a.unwrap();
    b.unwrap();
    worker::tick(&app.state, true).await.unwrap();

    let events: Vec<(String, String)> = sqlx::query_as(
        "SELECT event_type, status FROM notification_events WHERE schedule_id = $1::uuid AND event_type = 'rotation_changed' AND scheduled_for <= now()",
    )
    .bind(&schedule_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(events, vec![("rotation_changed".to_string(), "dispatched".to_string())]);
    let history = deliveries(&app, &owner).await;
    assert_eq!(history.len(), 1, "{history:?}");
    assert_eq!(history[0]["status"], "succeeded");
    assert_eq!(history[0]["event_type"], "rotation_changed");

    let received = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert!(body["embeds"][0]["title"].as_str().unwrap().contains("SRE"));
    assert_eq!(body["allowed_mentions"]["parse"], json!([]));
    assert!(received[0].url.query().unwrap_or("").contains("wait=true"));
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn transient_failures_are_retried_with_backoff(pool: PgPool) {
    let server = mock_discord().await;
    Mock::given(method("POST")).and(path(WEBHOOK_PATH)).respond_with(ResponseTemplate::new(503)).up_to_n_times(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path(WEBHOOK_PATH))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "2"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path(WEBHOOK_PATH)).respond_with(ResponseTemplate::new(204)).mount(&server).await;
    let app = app_for(pool.clone(), &server);
    let (owner, _, _) = setup(&app, &["rotation_changed"]).await;

    worker::tick(&app.state, true).await.unwrap();
    let d = &deliveries(&app, &owner).await[0];
    assert_eq!(d["status"], "pending");
    assert_eq!(d["attempts"], 1);
    assert_eq!(d["last_status_code"], 503);
    let next: chrono::DateTime<Utc> = d["next_attempt_at"].as_str().unwrap().parse().unwrap();
    assert!(next > Utc::now() + Duration::seconds(10), "backoff should delay the retry");

    // Time passes.
    sqlx::query("UPDATE notification_deliveries SET next_attempt_at = now()").execute(&pool).await.unwrap();
    worker::tick(&app.state, false).await.unwrap();
    let d = &deliveries(&app, &owner).await[0];
    assert_eq!(d["last_status_code"], 429);
    let next: chrono::DateTime<Utc> = d["next_attempt_at"].as_str().unwrap().parse().unwrap();
    assert!(next <= Utc::now() + Duration::seconds(3), "Retry-After must be respected");

    sqlx::query("UPDATE notification_deliveries SET next_attempt_at = now()").execute(&pool).await.unwrap();
    worker::tick(&app.state, false).await.unwrap();
    let d = &deliveries(&app, &owner).await[0];
    assert_eq!(d["status"], "succeeded");
    assert_eq!(d["attempts"], 3);
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM notification_delivery_attempts").fetch_one(&pool).await.unwrap();
    assert_eq!(attempts, 3);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn permanent_failure_stops_and_reports(pool: PgPool) {
    let server = mock_discord().await;
    Mock::given(method("POST")).and(path(WEBHOOK_PATH)).respond_with(ResponseTemplate::new(404)).mount(&server).await;
    let app = app_for(pool.clone(), &server);
    let (owner, _, _) = setup(&app, &["rotation_changed", "delivery_failed"]).await;
    worker::tick(&app.state, true).await.unwrap();
    let history = deliveries(&app, &owner).await;
    assert_eq!(history.len(), 1, "the failure report must not be sent to the integration that failed");
    assert_eq!(history[0]["status"], "failed");
    let failure_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notification_events WHERE event_type = 'delivery_failed'").fetch_one(&pool).await.unwrap();
    assert_eq!(failure_events, 1);
    let (status, _) = app.post(&format!("/api/v1/notifications/deliveries/{}/retry", history[0]["id"].as_str().unwrap()), &owner, json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn work_survives_a_worker_restart(pool: PgPool) {
    let server = mock_discord().await;
    Mock::given(method("POST")).and(path(WEBHOOK_PATH)).respond_with(ResponseTemplate::new(204)).expect(1).mount(&server).await;
    let app = app_for(pool.clone(), &server);
    let (owner, _, _) = setup(&app, &["rotation_changed"]).await;

    // Worker 1 plans and fans out, then "crashes" while holding a delivery lease.
    oncallkit::notifications::planner::plan_all(&app.state).await.unwrap();
    oncallkit::notifications::dispatcher::dispatch_due_events(&app.state).await.unwrap();
    sqlx::query("UPDATE notification_deliveries SET status = 'sending', attempts = 1, locked_until = now() + interval '60 seconds'")
        .execute(&pool)
        .await
        .unwrap();

    // A fresh process (new state, same database) does not steal a live lease…
    let uri: url::Url = server.uri().parse().unwrap();
    let mut config = oncallkit::config::Config::for_tests("postgres://unused");
    config.discord_base_override = Some(uri);
    let restarted = AppState::with_pool(pool.clone(), config, None);
    worker::tick(&restarted, true).await.unwrap();
    assert_eq!(deliveries(&app, &owner).await[0]["status"], "sending");

    // …but picks it up once the lease expires.
    sqlx::query("UPDATE notification_deliveries SET locked_until = now() - interval '1 second'").execute(&pool).await.unwrap();
    worker::tick(&restarted, true).await.unwrap();
    let d = &deliveries(&app, &owner).await[0];
    assert_eq!(d["status"], "succeeded");
    assert_eq!(d["attempts"], 2);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn schedule_changes_supersede_pending_reminders(pool: PgPool) {
    let server = mock_discord().await;
    let app = app_for(pool.clone(), &server);
    let (owner, schedule_id, _) = setup(&app, &["rotation_approaching"]).await;
    oncallkit::notifications::planner::plan_all(&app.state).await.unwrap();
    let pending = |pool: PgPool, id: String| async move {
        sqlx::query_scalar::<_, String>("SELECT dedupe_key FROM notification_events WHERE schedule_id = $1::uuid AND status = 'pending' AND event_type = 'rotation_approaching' ORDER BY dedupe_key")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap()
    };
    let before = pending(pool.clone(), schedule_id.clone()).await;
    assert!(!before.is_empty());

    // Pausing a schedule withdraws its pending reminders; reactivating restores them.
    app.patch(&format!("/api/v1/schedules/{schedule_id}"), &owner, json!({ "status": "paused" })).await;
    assert!(pending(pool.clone(), schedule_id.clone()).await.is_empty());
    app.patch(&format!("/api/v1/schedules/{schedule_id}"), &owner, json!({ "status": "active" })).await;
    assert_eq!(pending(pool.clone(), schedule_id.clone()).await, before);
}
