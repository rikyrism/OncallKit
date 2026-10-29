use std::net::IpAddr;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::{complete_login, create_session, expired_cookie, session_cookie, ClientIp, CurrentUser, LoginError, VerifiedIdentity};
use crate::{
    crypto::{hash_token, random_token},
    error::{AppError, AppResult},
    rbac::Role,
    state::AppState,
};

const OAUTH_COOKIE: &str = "oncallkit_oauth";
const OAUTH_COOKIE_PATH: &str = "/api/v1/auth";

/// Only same-site relative paths are accepted as post-login destinations (no open redirects).
pub fn safe_return_to(value: Option<&str>) -> String {
    match value {
        Some(v) if v.starts_with('/') && !v.starts_with("//") && !v.contains('\\') && !v.contains("://") && v.len() < 512 => v.to_string(),
        _ => "/".into(),
    }
}

#[derive(Serialize)]
pub struct Providers {
    google: bool,
    dev_login: bool,
}

pub async fn providers(State(state): State<AppState>) -> Json<Providers> {
    Json(Providers { google: state.google.is_some(), dev_login: state.config.dev_login })
}

#[derive(Deserialize)]
pub struct StartQuery {
    return_to: Option<String>,
}

pub async fn google_start(
    State(state): State<AppState>,
    _headers: HeaderMap,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    Query(query): Query<StartQuery>,
) -> AppResult<Response> {
    let google = state.google.clone().ok_or_else(|| AppError::BadRequest("Google sign-in is not configured".into()))?;
    if !state.auth_limiter.check(ip, "auth") {
        return Err(AppError::RateLimited);
    }
    let request = google.authorize();
    let binding = random_token();
    sqlx::query(
        "INSERT INTO oauth_flows (provider, state_hash, binding_hash, nonce, pkce_verifier, return_to, expires_at)
         VALUES ('google', $1, $2, $3, $4, $5, now() + interval '10 minutes')",
    )
    .bind(hash_token(&request.state))
    .bind(hash_token(&binding))
    .bind(&request.nonce)
    .bind(&request.pkce_verifier)
    .bind(safe_return_to(query.return_to.as_deref()))
    .execute(&state.db)
    .await?;
    sqlx::query("DELETE FROM oauth_flows WHERE expires_at < now() - interval '1 hour'").execute(&state.db).await?;

    let mut cookie = Cookie::new(OAUTH_COOKIE, binding);
    cookie.set_http_only(true);
    cookie.set_secure(state.config.secure_cookies());
    // Lax so the cookie survives the top-level redirect back from Google.
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path(OAUTH_COOKIE_PATH);
    cookie.set_max_age(time::Duration::minutes(10));
    Ok((jar.add(cookie), Redirect::to(&request.url)).into_response())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn login_error_redirect(state: &AppState, jar: CookieJar, code: &str) -> Response {
    let jar = jar.add(expired_cookie(OAUTH_COOKIE, OAUTH_COOKIE_PATH));
    (jar, Redirect::to(&state.config.app_url(&format!("/login?error={code}")))).into_response()
}

pub async fn google_callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    Query(query): Query<CallbackQuery>,
) -> AppResult<Response> {
    if !state.auth_limiter.check(ip, "auth") {
        return Err(AppError::RateLimited);
    }
    let Some(google) = state.google.clone() else {
        return Ok(login_error_redirect(&state, jar, "not_configured"));
    };
    if query.error.is_some() {
        return Ok(login_error_redirect(&state, jar, "access_denied"));
    }
    let (Some(code), Some(oauth_state)) = (query.code, query.state) else {
        return Ok(login_error_redirect(&state, jar, "invalid_request"));
    };

    // The flow is single-use: delete it whether or not the rest succeeds.
    let flow: Option<(Vec<u8>, String, String, String, DateTime<Utc>)> =
        sqlx::query_as("DELETE FROM oauth_flows WHERE state_hash = $1 RETURNING binding_hash, nonce, pkce_verifier, return_to, expires_at")
            .bind(hash_token(&oauth_state))
            .fetch_optional(&state.db)
            .await?;
    let Some((binding_hash, nonce, verifier, return_to, expires_at)) = flow else {
        return Ok(login_error_redirect(&state, jar, "invalid_state"));
    };
    let binding_ok = jar.get(OAUTH_COOKIE).is_some_and(|c| crate::observability::constant_time_eq(&hash_token(c.value()), &binding_hash));
    if !binding_ok || expires_at < Utc::now() {
        return Ok(login_error_redirect(&state, jar, "invalid_state"));
    }

    let identity = match google.exchange(code, verifier, nonce).await {
        Ok(identity) => identity,
        Err(err) => {
            tracing::warn!(error = %err, "Google sign-in failed");
            return Ok(login_error_redirect(&state, jar, "verification_failed"));
        }
    };
    finish_login(&state, jar, &headers, ip, &identity, &return_to).await
}

async fn finish_login(state: &AppState, jar: CookieJar, headers: &HeaderMap, ip: IpAddr, identity: &VerifiedIdentity, return_to: &str) -> AppResult<Response> {
    match complete_login(&state.db, &state.config, identity).await? {
        Ok(user_id) => {
            let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok());
            let session = create_session(&state.db, &state.config, user_id, ua, Some(ip)).await?;
            tracing::info!(%user_id, provider = %identity.provider, "user signed in");
            let jar = jar.add(expired_cookie(OAUTH_COOKIE, OAUTH_COOKIE_PATH)).add(session_cookie(&state.config, session.token));
            Ok((jar, Redirect::to(&state.config.app_url(return_to))).into_response())
        }
        Err(err) => {
            tracing::info!(reason = err.code(), provider = %identity.provider, "sign-in refused");
            Ok(login_error_redirect(state, jar, err.code()))
        }
    }
}

#[derive(Deserialize)]
pub struct DevLoginRequest {
    email: String,
    name: Option<String>,
}

/// Development-only sign-in, enabled with AUTH_DEV_LOGIN=true and refused at startup in
/// production. It still goes through the registration policy and creates a normal session.
pub async fn dev_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    Json(body): Json<DevLoginRequest>,
) -> AppResult<Response> {
    if !state.config.dev_login || state.config.is_production() {
        return Err(AppError::NotFound);
    }
    if !state.auth_limiter.check(ip, "auth") {
        return Err(AppError::RateLimited);
    }
    let email = body.email.trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() > 254 {
        return Err(AppError::validation("email", "enter an email address"));
    }
    let identity = VerifiedIdentity { provider: "dev".into(), subject: email.clone(), email, email_verified: true, name: body.name, picture: None };
    match complete_login(&state.db, &state.config, &identity).await? {
        Ok(user_id) => {
            let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok());
            let session = create_session(&state.db, &state.config, user_id, ua, Some(ip)).await?;
            Ok((jar.add(session_cookie(&state.config, session.token)), Json(json!({ "ok": true }))).into_response())
        }
        Err(LoginError::RegistrationClosed) => Err(AppError::Forbidden("registration is closed for this address".into())),
        Err(err) => Err(AppError::Forbidden(err.to_string())),
    }
}

pub async fn logout(State(state): State<AppState>, user: CurrentUser, jar: CookieJar) -> AppResult<Response> {
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1").bind(user.session_id).execute(&state.db).await?;
    let jar = jar.add(expired_cookie(state.config.session_cookie_name(), "/"));
    Ok((jar, StatusCode::NO_CONTENT).into_response())
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MembershipView {
    organization_id: Uuid,
    name: String,
    slug: String,
    role: String,
}

#[derive(Serialize)]
pub struct MeResponse {
    user: CurrentUser,
    csrf_token: String,
    memberships: Vec<MembershipView>,
    active_organization_id: Option<Uuid>,
    role: Option<String>,
    permissions: Vec<&'static str>,
    can_create_organization: bool,
}

pub async fn me(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<MeResponse>> {
    let memberships: Vec<MembershipView> = sqlx::query_as(
        "SELECT o.id AS organization_id, o.name, o.slug, m.role FROM organization_memberships m
         JOIN organizations o ON o.id = m.organization_id WHERE m.user_id = $1 ORDER BY o.name",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    let role = user.active_organization_id.and_then(|org| memberships.iter().find(|m| m.organization_id == org)).and_then(|m| Role::parse(&m.role));
    let active = role.and(user.active_organization_id);
    Ok(Json(MeResponse {
        csrf_token: user.csrf_token.clone(),
        active_organization_id: active,
        role: role.map(|r| r.as_str().to_string()),
        permissions: role.map(|r| r.permissions()).unwrap_or_default(),
        can_create_organization: user.is_instance_admin || state.config.org_creation == crate::config::OrgCreation::Anyone,
        memberships,
        user,
    }))
}

#[derive(Deserialize)]
pub struct UpdateProfile {
    display_name: Option<String>,
    /// Discord user id (the numeric snowflake), used for optional @mentions. Empty clears it.
    discord_user_id: Option<String>,
}

pub async fn update_profile(State(state): State<AppState>, user: CurrentUser, Json(body): Json<UpdateProfile>) -> AppResult<StatusCode> {
    if let Some(name) = &body.display_name {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(AppError::validation("display_name", "use 1 to 120 characters"));
        }
        sqlx::query("UPDATE users SET display_name = $2, updated_at = now() WHERE id = $1").bind(user.user_id).bind(name).execute(&state.db).await?;
    }
    if let Some(discord) = &body.discord_user_id {
        let discord = discord.trim();
        let value = if discord.is_empty() { None } else { Some(discord) };
        if value.is_some_and(|v| !(5..=25).contains(&v.len()) || !v.chars().all(|c| c.is_ascii_digit())) {
            return Err(AppError::validation("discord_user_id", "a Discord user id is a number of 5 to 25 digits"));
        }
        sqlx::query("UPDATE users SET discord_user_id = $2, updated_at = now() WHERE id = $1").bind(user.user_id).bind(value).execute(&state.db).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SessionView {
    id: Uuid,
    user_agent: Option<String>,
    ip_address: Option<String>,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    #[sqlx(default)]
    current: bool,
}

pub async fn list_sessions(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Vec<SessionView>>> {
    let mut sessions: Vec<SessionView> = sqlx::query_as(
        "SELECT id, user_agent, ip_address, created_at, last_seen_at, expires_at FROM sessions
         WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now() ORDER BY last_seen_at DESC",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    for s in &mut sessions {
        s.current = s.id == user.session_id;
    }
    Ok(Json(sessions))
}

pub async fn revoke_session(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let result = sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL")
        .bind(id)
        .bind(user.user_id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Remove expired sessions and OAuth flows. Called periodically by the worker.
pub async fn purge_expired(db: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE expires_at < now() - interval '30 days' OR revoked_at < now() - interval '30 days'").execute(db).await?;
    sqlx::query("DELETE FROM oauth_flows WHERE expires_at < now()").execute(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::safe_return_to;

    #[test]
    fn return_to_rejects_open_redirects() {
        assert_eq!(safe_return_to(Some("/schedules/1")), "/schedules/1");
        assert_eq!(safe_return_to(Some("//evil.com")), "/");
        assert_eq!(safe_return_to(Some("https://evil.com")), "/");
        assert_eq!(safe_return_to(Some("/\\evil.com")), "/");
        assert_eq!(safe_return_to(Some("evil")), "/");
        assert_eq!(safe_return_to(None), "/");
    }
}
