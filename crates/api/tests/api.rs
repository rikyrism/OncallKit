mod common;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use common::TestApp;
use oncallkit::config::RegistrationMode;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn unauthenticated_requests_are_rejected(pool: PgPool) {
    let app = TestApp::new(pool);
    let (status, body) = app.request(Method::GET, "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthenticated");
    let (status, _) = app.request(Method::GET, "/api/v1/schedules", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = app.request(Method::GET, "/healthz", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.request(Method::GET, "/readyz", None, None).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn session_lifecycle_and_csrf(pool: PgPool) {
    let app = TestApp::new(pool);
    let s = app.login("owner@acme.test").await;

    // A mutation without the CSRF header is refused even with a valid cookie.
    let mut no_csrf = s.clone();
    no_csrf.csrf = "wrong".into();
    let (status, _) = app.post("/api/v1/organizations", &no_csrf, json!({ "name": "Acme" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, sessions) = app.get("/api/v1/me/sessions", &s).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["current"], true);

    // Signing in again creates a different session (identifier rotation on login).
    let s2 = app.login("owner@acme.test").await;
    assert_ne!(s.cookie, s2.cookie);

    let (status, _) = app.request(Method::POST, "/api/v1/auth/logout", Some(&s), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.get("/api/v1/me", &s).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "logged-out session must be invalid server-side");
    let (status, _) = app.get("/api/v1/me", &s2).await;
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn expired_sessions_are_rejected(pool: PgPool) {
    let app = TestApp::new(pool.clone());
    let s = app.login("a@acme.test").await;
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    let (status, _) = app.get("/api/v1/me", &s).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn invite_only_registration(pool: PgPool) {
    let app = TestApp::with_config(pool.clone(), |c| c.registration = RegistrationMode::InviteOnly);
    assert_eq!(app.try_login("stranger@acme.test").await.err(), Some(StatusCode::FORBIDDEN));

    // The bootstrap grant lets the first administrator in.
    sqlx::query("INSERT INTO instance_admin_grants (email) VALUES ('admin@acme.test')").execute(&pool).await.unwrap();
    let admin = app.login("admin@acme.test").await;
    assert_eq!(app.me(&admin).await["user"]["is_instance_admin"], true);

    let (status, _) = app.post("/api/v1/organizations", &admin, json!({ "name": "Acme" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, inv) = app.post("/api/v1/invitations", &admin, json!({ "email": "new@acme.test", "role": "member" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = inv["invite_url"].as_str().unwrap().rsplit('/').next().unwrap().to_string();

    // Invitations are bound to their email address.
    let other = app.try_login("stranger@acme.test").await;
    assert!(other.is_err());
    let invited = app.login("new@acme.test").await;
    let (status, _) = app.post("/api/v1/invitations/accept", &invited, json!({ "token": token })).await;
    assert_eq!(status, StatusCode::OK);
    let me = app.me(&invited).await;
    assert_eq!(me["role"], "member");
    // A used invitation cannot be reused.
    let (status, _) = app.post("/api/v1/invitations/accept", &invited, json!({ "token": token })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn domain_registration(pool: PgPool) {
    let app = TestApp::with_config(pool, |c| c.registration = RegistrationMode::Domains(vec!["acme.test".into()]));
    assert!(app.try_login("ok@acme.test").await.is_ok());
    assert_eq!(app.try_login("no@evil.test").await.err(), Some(StatusCode::FORBIDDEN));
}

fn weekly_schedule(primary: &[&str]) -> serde_json::Value {
    json!({
        "name": "DevOps On-call",
        "timezone": "Asia/Jakarta",
        "start_local": "2026-11-03T09:00",
        "end_local": "2027-02-03T09:00",
        "rotation": { "interval": 7, "unit": "days", "primary": primary, "secondary": [] },
    })
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn schedule_crud_and_rotation(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.login("riky@acme.test").await;
    let people = app.org_with_members(&owner, &[("andi@acme.test", "member"), ("budi@acme.test", "scheduler")]).await;
    let riky = app.user_id(&owner).await;
    let (andi, budi) = (people[0].1.clone(), people[1].1.clone());

    let (status, team) = app.post("/api/v1/teams", &owner, json!({ "name": "DevOps", "member_ids": [riky, andi, budi] })).await;
    assert_eq!(status, StatusCode::CREATED, "{team}");
    assert_eq!(team["member_ids"].as_array().unwrap().len(), 3);

    let mut body = weekly_schedule(&[&riky, &andi, &budi]);
    body["team_id"] = team["id"].clone();

    let (status, preview) = app.post("/api/v1/schedules/preview", &owner, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["start_at"], "2026-11-03T02:00:00Z");
    let users: Vec<&str> = preview["shifts"].as_array().unwrap().iter().take(4).map(|s| s["user_id"].as_str().unwrap()).collect();
    assert_eq!(users, vec![riky.as_str(), andi.as_str(), budi.as_str(), riky.as_str()]);

    let (status, created) = app.post("/api/v1/schedules", &owner, body.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["start_local"], "2026-11-03T09:00");

    let (_, oc) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at=2026-11-10T02:00:00Z"), &owner).await;
    assert_eq!(oc["primary"]["user_id"], andi.as_str());
    assert_eq!(oc["next_rotation_at"], "2026-11-17T02:00:00Z");
    let (_, oc) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at=2026-11-10T01:59:59Z"), &owner).await;
    assert_eq!(oc["primary"]["user_id"], riky.as_str());
    let (_, oc) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at=2027-02-03T02:00:00Z"), &owner).await;
    assert!(oc["primary"].is_null(), "schedule has ended");

    let (status, shifts) = app.get(&format!("/api/v1/schedules/{id}/assignments?from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z"), &owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shifts.as_array().unwrap().len(), 4);

    // Validation errors.
    let mut bad = body.clone();
    bad["end_local"] = json!("2026-11-01T09:00");
    let (status, err) = app.post("/api/v1/schedules", &owner, bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["error"]["field"], "end_local");
    let mut empty = body.clone();
    empty["rotation"]["primary"] = json!([]);
    let (status, _) = app.post("/api/v1/schedules", &owner, empty).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Overrides replace the assignee for their window.
    let (status, o) = app
        .post(
            &format!("/api/v1/schedules/{id}/overrides"),
            &owner,
            json!({ "layer": "primary", "user_id": budi, "start_local": "2026-11-11T00:00", "end_local": "2026-11-12T00:00", "reason": "Leave" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{o}");
    let (_, oc) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at=2026-11-11T05:00:00Z"), &owner).await;
    assert_eq!(oc["primary"]["user_id"], budi.as_str());
    assert_eq!(oc["primary"]["source"]["kind"], "override");
    let (status, _) = app.delete(&format!("/api/v1/schedules/{id}/overrides/{}", o["id"].as_str().unwrap()), &owner).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, oc) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at=2026-11-11T05:00:00Z"), &owner).await;
    assert_eq!(oc["primary"]["user_id"], andi.as_str());

    // Members can read but not change schedules; schedulers can.
    let member = &people[0].0;
    let (status, _) = app.get(&format!("/api/v1/schedules/{id}"), member).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app.patch(&format!("/api/v1/schedules/{id}"), member, json!({ "name": "Hacked" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let scheduler = &people[1].0;
    let (status, updated) = app.patch(&format!("/api/v1/schedules/{id}"), scheduler, json!({ "name": "DevOps Primary" })).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "DevOps Primary");
    let (status, _) = app.post("/api/v1/teams", scheduler, json!({ "name": "Nope" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Archive keeps history but stops on-call.
    let (status, _) = app.delete(&format!("/api/v1/schedules/{id}"), &owner).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = app.get("/api/v1/schedules", &owner).await;
    assert!(list.as_array().unwrap().is_empty());
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn editing_an_active_schedule_keeps_history(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.login("riky@acme.test").await;
    let people = app.org_with_members(&owner, &[("andi@acme.test", "member"), ("budi@acme.test", "member")]).await;
    let riky = app.user_id(&owner).await;
    let (andi, budi) = (people[0].1.clone(), people[1].1.clone());

    let start = Utc::now() - Duration::days(10);
    let tz: chrono_tz::Tz = "Asia/Jakarta".parse().unwrap();
    let start_local = start.with_timezone(&tz).format("%Y-%m-%dT%H:%M").to_string();
    let (status, created) = app
        .post("/api/v1/schedules", &owner, json!({ "name": "Daily", "timezone": "Asia/Jakarta", "start_local": start_local, "rotation": { "interval": 1, "unit": "days", "primary": [riky, andi, budi] } }))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let past = (start + Duration::days(4) + Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (_, before) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at={past}"), &owner).await;

    // Starting point and timezone are frozen once the schedule has started.
    let (status, _) = app.patch(&format!("/api/v1/schedules/{id}"), &owner, json!({ "timezone": "UTC" })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, updated) = app
        .patch(&format!("/api/v1/schedules/{id}"), &owner, json!({ "rotation": { "interval": 1, "unit": "days", "primary": [budi] }, "apply_from": "now" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["versions"].as_array().unwrap().len(), 2);
    assert_eq!(updated["on_call"]["primary"]["user_id"], budi.as_str());
    let (_, after) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at={past}"), &owner).await;
    assert_eq!(before["primary"], after["primary"], "past assignments must not change");

    // Removing a member takes them out of rotations from now on, keeping history.
    let (status, _) = app.delete(&format!("/api/v1/members/{budi}"), &owner).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, detail) = app.get(&format!("/api/v1/schedules/{id}"), &owner).await;
    assert!(detail["on_call"]["primary"].is_null());
    let (_, after) = app.get(&format!("/api/v1/schedules/{id}/on-call-now?at={past}"), &owner).await;
    assert_eq!(before["primary"], after["primary"]);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn tenants_are_isolated(pool: PgPool) {
    let app = TestApp::new(pool);
    let a = app.login("a@one.test").await;
    let b = app.login("b@two.test").await;
    app.post("/api/v1/organizations", &a, json!({ "name": "One" })).await;
    app.post("/api/v1/organizations", &b, json!({ "name": "Two" })).await;
    let a_id = app.user_id(&a).await;
    let b_id = app.user_id(&b).await;

    let (_, schedule) = app.post("/api/v1/schedules", &a, weekly_schedule(&[&a_id])).await;
    let sid = schedule["id"].as_str().unwrap();
    let (_, team) = app.post("/api/v1/teams", &a, json!({ "name": "Ops" })).await;

    // Org two cannot see or touch org one's resources.
    assert_eq!(app.get(&format!("/api/v1/schedules/{sid}"), &b).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.patch(&format!("/api/v1/schedules/{sid}"), &b, json!({ "name": "x" })).await.0, StatusCode::NOT_FOUND);
    assert_eq!(app.get(&format!("/api/v1/teams/{}", team["id"].as_str().unwrap()), &b).await.0, StatusCode::NOT_FOUND);
    let a_org = app.me(&a).await["active_organization_id"].as_str().unwrap().to_string();
    assert_eq!(app.request(Method::POST, &format!("/api/v1/organizations/{a_org}/activate"), Some(&b), None).await.0, StatusCode::NOT_FOUND);
    // Nor reference people from another organization.
    let (status, _) = app.post("/api/v1/schedules", &a, weekly_schedule(&[&b_id])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // And a team id from another org is rejected.
    let mut s = weekly_schedule(&[&b_id]);
    s["team_id"] = team["id"].clone();
    assert_eq!(app.post("/api/v1/schedules", &b, s).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrator = "oncallkit::MIGRATOR")]
async fn role_management_rules(pool: PgPool) {
    let app = TestApp::new(pool);
    let owner = app.login("owner@acme.test").await;
    let people = app.org_with_members(&owner, &[("admin@acme.test", "admin"), ("m@acme.test", "member")]).await;
    let owner_id = app.user_id(&owner).await;
    let admin = &people[0].0;
    let member_id = &people[1].1;

    // Admins cannot create admins or remove the owner.
    assert_eq!(app.patch(&format!("/api/v1/members/{member_id}"), admin, json!({ "role": "admin" })).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.patch(&format!("/api/v1/members/{member_id}"), admin, json!({ "role": "scheduler" })).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.delete(&format!("/api/v1/members/{owner_id}"), admin).await.0, StatusCode::CONFLICT);
    assert_eq!(app.post("/api/v1/organizations/current/transfer-ownership", admin, json!({ "user_id": member_id })).await.0, StatusCode::FORBIDDEN);
    // Audit log is visible to admins, not to members.
    assert_eq!(app.get("/api/v1/audit-logs", admin).await.0, StatusCode::OK);
    assert_eq!(app.get("/api/v1/audit-logs", &people[1].0).await.0, StatusCode::FORBIDDEN);
    let (_, log) = app.get("/api/v1/audit-logs?action=member.", &owner).await;
    assert!(log["items"].as_array().unwrap().iter().any(|e| e["action"] == "member.role_changed"));
}
