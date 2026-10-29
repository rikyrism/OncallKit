#![allow(dead_code)]

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use oncallkit::{config::Config, routes, state::AppState};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

pub const WEBHOOK_PATH: &str = "/api/webhooks/123456789012345678/abcdefghijklmnopqrstuvwxyz_0123456789";

pub fn webhook_url() -> String {
    format!("https://discord.com{WEBHOOK_PATH}")
}

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
}

#[derive(Clone)]
pub struct Session {
    pub cookie: String,
    pub csrf: String,
}

impl TestApp {
    pub fn new(pool: PgPool) -> Self {
        Self::with_config(pool, |_| {})
    }

    pub fn with_config(pool: PgPool, f: impl FnOnce(&mut Config)) -> Self {
        let mut config = Config::for_tests("postgres://unused");
        f(&mut config);
        let state = AppState::with_pool(pool, config, None);
        TestApp { router: routes::router(state.clone()), state }
    }

    pub async fn request(&self, method: Method, uri: &str, session: Option<&Session>, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method.clone()).uri(uri);
        if let Some(s) = session {
            req = req.header(header::COOKIE, &s.cookie);
            if method != Method::GET {
                req = req.header("x-csrf-token", &s.csrf);
            }
        }
        let req = match body {
            Some(b) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())).unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into())) };
        (status, json)
    }

    pub async fn get(&self, uri: &str, s: &Session) -> (StatusCode, Value) {
        self.request(Method::GET, uri, Some(s), None).await
    }

    pub async fn post(&self, uri: &str, s: &Session, body: Value) -> (StatusCode, Value) {
        self.request(Method::POST, uri, Some(s), Some(body)).await
    }

    pub async fn patch(&self, uri: &str, s: &Session, body: Value) -> (StatusCode, Value) {
        self.request(Method::PATCH, uri, Some(s), Some(body)).await
    }

    pub async fn delete(&self, uri: &str, s: &Session) -> (StatusCode, Value) {
        self.request(Method::DELETE, uri, Some(s), None).await
    }

    /// Sign in through the development login, which runs the same registration policy and
    /// session creation as Google sign-in.
    pub async fn try_login(&self, email: &str) -> Result<Session, StatusCode> {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/auth/dev/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({ "email": email, "name": email.split('@').next().unwrap() }).to_string()))
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        if res.status() != StatusCode::OK {
            return Err(res.status());
        }
        let set_cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .find(|c| c.starts_with("oncallkit_session="))
            .expect("session cookie");
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains("SameSite=Lax"));
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let mut session = Session { cookie, csrf: String::new() };
        let (status, me) = self.get("/api/v1/me", &session).await;
        assert_eq!(status, StatusCode::OK);
        session.csrf = me["csrf_token"].as_str().unwrap().to_string();
        Ok(session)
    }

    pub async fn login(&self, email: &str) -> Session {
        self.try_login(email).await.expect("login")
    }

    pub async fn me(&self, s: &Session) -> Value {
        self.get("/api/v1/me", s).await.1
    }

    pub async fn user_id(&self, s: &Session) -> String {
        self.me(s).await["user"]["user_id"].as_str().unwrap().to_string()
    }

    /// Create an organization owned by `owner` and add the given people as members via invitations.
    pub async fn org_with_members(&self, owner: &Session, members: &[(&str, &str)]) -> Vec<(Session, String)> {
        let (status, _) = self.post("/api/v1/organizations", owner, serde_json::json!({ "name": "Acme", "timezone": "Asia/Jakarta" })).await;
        assert_eq!(status, StatusCode::CREATED);
        let mut out = Vec::new();
        for (email, role) in members {
            let (status, inv) = self.post("/api/v1/invitations", owner, serde_json::json!({ "email": email, "role": role })).await;
            assert_eq!(status, StatusCode::CREATED, "{inv}");
            let token = inv["invite_url"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
            let s = self.login(email).await;
            let (status, body) = self.post("/api/v1/invitations/accept", &s, serde_json::json!({ "token": token })).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let id = self.user_id(&s).await;
            out.push((s, id));
        }
        out
    }
}
