mod common;

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use common::{Session, TestApp};
use serde_json::{json, Value};
use sqlx::PgPool;

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

struct Setup {
    owner: Session,
    andi: Session,
    budi: Session,
    riky_id: String,
    andi_id: String,
    budi_id: String,
    schedule: String,
    /// Shift n of the weekly rotation starts at start + n weeks. Riky holds even shifts, Andi odd.
    start: DateTime<Utc>,
}

impl Setup {
    fn shift(&self, n: i64) -> (DateTime<Utc>, DateTime<Utc>) {
        (self.start + Duration::weeks(n), self.start + Duration::weeks(n + 1))
    }
}

async fn setup(app: &TestApp) -> Setup {
    let owner = app.login("riky@acme.test").await;
    let people = app.org_with_members(&owner, &[("andi@acme.test", "member"), ("budi@acme.test", "member")]).await;
    let riky_id = app.user_id(&owner).await;
    let start_local = (Utc::now() - Duration::days(3)).format("%Y-%m-%dT%H:%M").to_string();
    let (status, created) = app
        .post(
            "/api/v1/schedules",
            &owner,
            json!({ "name": "Weekly", "timezone": "UTC", "start_local": start_local, "rotation": { "interval": 7, "unit": "days", "primary": [riky_id, people[0].1] } }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    Setup {
        owner,
        andi: people[0].0.clone(),
        budi: people[1].0.clone(),
        riky_id,
        andi_id: people[0].1.clone(),
        budi_id: people[1].1.clone(),
        schedule: created["id"].as_str().unwrap().to_string(),
        start: created["start_at"].as_str().unwrap().parse().unwrap(),
    }
}

async fn on_call_at(app: &TestApp, s: &Setup, at: DateTime<Utc>) -> Value {
    let (_, body) = app.get(&format!("/api/v1/schedules/{}/on-call-now?at={}", s.schedule, iso(at)), &s.owner).await;
    body["primary"]["user_id"].clone()
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn an_accepted_swap_becomes_overrides(pool: PgPool) {
    let app = TestApp::new(pool);
    let s = setup(&app).await;
    let (mine, theirs) = (s.shift(2), s.shift(3));

    let (status, swap) = app
        .post(
            "/api/v1/swap-requests",
            &s.owner,
            json!({ "schedule_id": s.schedule, "layer": "primary", "target_user_id": s.andi_id, "start": iso(mine.0), "end": iso(mine.1),
                    "return_start": iso(theirs.0), "return_end": iso(theirs.1), "reason": "Family trip" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{swap}");
    assert_eq!(swap["status"], "pending");
    let id = swap["id"].as_str().unwrap();

    // Andi sees it waiting; Budi is not involved and does not.
    let (_, andi_list) = app.get("/api/v1/swap-requests", &s.andi).await;
    assert_eq!(andi_list.as_array().unwrap().len(), 1);
    let (_, budi_list) = app.get("/api/v1/swap-requests", &s.budi).await;
    assert!(budi_list.as_array().unwrap().is_empty());
    // Only schedulers may see everything.
    assert_eq!(app.get("/api/v1/swap-requests?all=true", &s.budi).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.get("/api/v1/swap-requests?all=true", &s.owner).await.1.as_array().unwrap().len(), 1);

    // Only the person asked can accept.
    assert_eq!(app.post(&format!("/api/v1/swap-requests/{id}/accept"), &s.budi, json!({})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.post(&format!("/api/v1/swap-requests/{id}/accept"), &s.owner, json!({})).await.0, StatusCode::FORBIDDEN);

    let (status, accepted) = app.post(&format!("/api/v1/swap-requests/{id}/accept"), &s.andi, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["status"], "accepted");
    assert_eq!(on_call_at(&app, &s, mine.0 + Duration::days(1)).await, s.andi_id.as_str());
    assert_eq!(on_call_at(&app, &s, theirs.0 + Duration::days(1)).await, s.riky_id.as_str());
    // The rotation itself is untouched before and after.
    assert_eq!(on_call_at(&app, &s, s.shift(4).0 + Duration::days(1)).await, s.riky_id.as_str());

    let (_, detail) = app.get(&format!("/api/v1/schedules/{}", s.schedule), &s.owner).await;
    let overrides = detail["overrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 2);
    assert!(overrides.iter().all(|o| o["reason"].as_str().unwrap().starts_with("Swap with ")));

    // Answering twice is refused.
    let (status, _) = app.post(&format!("/api/v1/swap-requests/{id}/accept"), &s.andi, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn swap_requests_are_validated(pool: PgPool) {
    let app = TestApp::new(pool);
    let s = setup(&app).await;
    let post = |body: Value| {
        let app = &app;
        let owner = s.owner.clone();
        async move { app.post("/api/v1/swap-requests", &owner, body).await }
    };
    let base = |(start, end): (DateTime<Utc>, DateTime<Utc>), target: &str| json!({ "schedule_id": s.schedule, "layer": "primary", "target_user_id": target, "start": iso(start), "end": iso(end) });

    // A shift that is not yours.
    assert_eq!(post(base(s.shift(1), &s.budi_id)).await.0, StatusCode::CONFLICT);
    // A shift that has already started.
    assert_eq!(post(base(s.shift(0), &s.andi_id)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // Yourself.
    assert_eq!(post(base(s.shift(2), &s.riky_id)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // Someone outside the organization.
    let outsider = app.login("eve@evil.test").await;
    let outsider_id = app.user_id(&outsider).await;
    assert_eq!(post(base(s.shift(2), &outsider_id)).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // An exchange shift that is not the colleague's.
    let mut body = base(s.shift(2), &s.andi_id);
    body["return_start"] = json!(iso(s.shift(4).0));
    body["return_end"] = json!(iso(s.shift(4).1));
    assert_eq!(post(body).await.0, StatusCode::CONFLICT);
    // Half an exchange.
    let mut body = base(s.shift(2), &s.andi_id);
    body["return_start"] = json!(iso(s.shift(3).0));
    assert_eq!(post(body).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn swaps_are_rechecked_when_accepted_and_can_be_closed(pool: PgPool) {
    let app = TestApp::new(pool);
    let s = setup(&app).await;
    let ask = |target: &str, n: i64| {
        let (start, end) = s.shift(n);
        json!({ "schedule_id": s.schedule, "layer": "primary", "target_user_id": target, "start": iso(start), "end": iso(end) })
    };

    // The schedule changes while the request waits: Riky no longer holds that shift.
    let (_, stale) = app.post("/api/v1/swap-requests", &s.owner, ask(&s.budi_id, 4)).await;
    let (_, declined) = app.post("/api/v1/swap-requests", &s.owner, ask(&s.andi_id, 2)).await;
    let (_, canceled) = app.post("/api/v1/swap-requests", &s.owner, ask(&s.andi_id, 6)).await;
    let (status, _) = app
        .patch(
            &format!("/api/v1/schedules/{}", s.schedule),
            &s.owner,
            json!({ "rotation": { "interval": 7, "unit": "days", "primary": [s.andi_id] }, "apply_from": "now" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = app.post(&format!("/api/v1/swap-requests/{}/accept", stale["id"].as_str().unwrap()), &s.budi, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (_, detail) = app.get(&format!("/api/v1/schedules/{}", s.schedule), &s.owner).await;
    assert!(detail["overrides"].as_array().unwrap().is_empty(), "a refused accept creates nothing");

    // Declining is for the person asked; cancelling for the person asking.
    let declined = declined["id"].as_str().unwrap();
    assert_eq!(app.post(&format!("/api/v1/swap-requests/{declined}/decline"), &s.owner, json!({})).await.0, StatusCode::FORBIDDEN);
    let (status, body) = app.post(&format!("/api/v1/swap-requests/{declined}/decline"), &s.andi, json!({})).await;
    assert_eq!((status, body["status"].as_str()), (StatusCode::OK, Some("declined")));
    let canceled = canceled["id"].as_str().unwrap();
    assert_eq!(app.post(&format!("/api/v1/swap-requests/{canceled}/cancel"), &s.andi, json!({})).await.0, StatusCode::FORBIDDEN);
    let (status, body) = app.post(&format!("/api/v1/swap-requests/{canceled}/cancel"), &s.owner, json!({})).await;
    assert_eq!((status, body["status"].as_str()), (StatusCode::OK, Some("canceled")));
    assert_eq!(app.post(&format!("/api/v1/swap-requests/{canceled}/accept"), &s.andi, json!({})).await.0, StatusCode::CONFLICT);

    // Removing a member cancels the requests they are part of.
    let (status, body) = app.delete(&format!("/api/v1/members/{}", s.budi_id), &s.owner).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, all) = app.get("/api/v1/swap-requests?all=true", &s.owner).await;
    let stale_now = all.as_array().unwrap().iter().find(|w| w["id"] == stale["id"]).unwrap();
    assert_eq!(stale_now["status"], "canceled");
}
