mod common;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use common::TestApp;
use serde_json::{json, Value};
use sqlx::PgPool;

/// Fetch a feed the way a calendar app does: no cookie, only the secret link.
async fn fetch(app: &TestApp, created: &Value) -> (StatusCode, String) {
    let url = created["url"].as_str().unwrap();
    let path = &url[url.find("/api/v1/").unwrap()..];
    let (status, body) = app.request(Method::GET, path, None, None).await;
    (status, body.as_str().map(str::to_string).unwrap_or_default())
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn calendar_feeds_follow_the_schedule_and_can_be_revoked(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.login("riky@acme.test").await;
    let people = app.org_with_members(&owner, &[("andi@acme.test", "member")]).await;
    let (andi_session, andi) = (people[0].0.clone(), people[0].1.clone());
    let riky = app.user_id(&owner).await;

    let start = (Utc::now() - Duration::days(3)).format("%Y-%m-%dT%H:%M").to_string();
    let (status, schedule) = app
        .post(
            "/api/v1/schedules",
            &owner,
            json!({ "name": "Weekly", "timezone": "UTC", "start_local": start, "rotation": { "interval": 7, "unit": "days", "primary": [riky, andi] } }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{schedule}");
    let schedule_id = schedule["id"].as_str().unwrap();

    // Andi's own shifts.
    let (status, mine) = app.post("/api/v1/calendar-feeds", &andi_session, json!({})).await;
    assert_eq!(status, StatusCode::CREATED, "{mine}");
    assert!(mine["webcal_url"].as_str().unwrap().starts_with("webcal://"));
    let (status, ics) = fetch(&app, &mine).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"), "{ics}");
    assert!(ics.contains("SUMMARY:On call: Weekly (primary)"));
    // Riky and Andi alternate weekly, so roughly half of the ~30 weeks in the window are Andi's.
    let andi_events = ics.matches("BEGIN:VEVENT").count();
    assert!((10..=20).contains(&andi_events), "{andi_events} events");

    // A whole schedule, with names.
    let (status, whole) = app.post("/api/v1/calendar-feeds", &owner, json!({ "schedule_id": schedule_id })).await;
    assert_eq!(status, StatusCode::CREATED, "{whole}");
    let (_, ics) = fetch(&app, &whole).await;
    assert!(ics.contains("SUMMARY:riky on call: Weekly (primary)"), "{ics}");
    assert!(ics.contains("SUMMARY:andi on call: Weekly (primary)"));
    assert!(ics.matches("BEGIN:VEVENT").count() > andi_events);

    // The link is shown once; listing never returns it.
    let (_, list) = app.get("/api/v1/calendar-feeds", &owner).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["schedule_name"], "Weekly");
    assert!(list[0].get("url").is_none());

    // Unknown or missing tokens reveal nothing.
    assert_eq!(app.request(Method::GET, "/api/v1/calendar.ics?token=nope", None, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.request(Method::GET, "/api/v1/calendar.ics", None, None).await.0, StatusCode::NOT_FOUND);

    // Revoking a link stops it at once.
    let (status, _) = app.delete(&format!("/api/v1/calendar-feeds/{}", whole["id"].as_str().unwrap()), &owner).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(fetch(&app, &whole).await.0, StatusCode::NOT_FOUND);

    // Someone else's link cannot be revoked.
    let (status, _) = app.delete(&format!("/api/v1/calendar-feeds/{}", mine["id"].as_str().unwrap()), &owner).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Leaving the organization kills the person's links.
    let (status, body) = app.delete(&format!("/api/v1/members/{andi}"), &owner).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(fetch(&app, &mine).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn calendar_feeds_stay_inside_the_organization(pool: PgPool) {
    let app = TestApp::new(pool);
    let a = app.login("a@one.test").await;
    let b = app.login("b@two.test").await;
    app.post("/api/v1/organizations", &a, json!({ "name": "One" })).await;
    app.post("/api/v1/organizations", &b, json!({ "name": "Two" })).await;
    let a_id = app.user_id(&a).await;
    let (_, schedule) = app
        .post("/api/v1/schedules", &a, json!({ "name": "Private", "timezone": "UTC", "start_local": "2026-11-03T09:00", "rotation": { "interval": 1, "unit": "days", "primary": [a_id] } }))
        .await;
    let (status, _) = app.post("/api/v1/calendar-feeds", &b, json!({ "schedule_id": schedule["id"] })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
