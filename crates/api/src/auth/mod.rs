//! Authentication: identity linking, registration policy, server-side sessions and the request
//! extractors every protected handler uses.
//!
//! Sessions are opaque random tokens kept in an HttpOnly cookie; only their SHA-256 hash is
//! stored. A fresh session is created at every sign-in (no pre-login session exists to fixate).
//! State-changing requests must echo the session's CSRF token in `X-CSRF-Token`.

pub mod google;
pub mod routes;

use std::net::{IpAddr, SocketAddr};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::{request::Parts, Method},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    config::{Config, RegistrationMode},
    crypto::{hash_token, random_token},
    error::{AppError, AppResult},
    observability::constant_time_eq,
    rbac::{Permission, Role},
    state::AppState,
};

pub const CSRF_HEADER: &str = "x-csrf-token";

#[derive(Clone, Debug)]
pub struct VerifiedIdentity {
    pub provider: String,
    pub subject: String,
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
    pub picture: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LoginError {
    #[error("the email address is not verified by the identity provider")]
    EmailNotVerified,
    #[error("registration is closed for this address")]
    RegistrationClosed,
    #[error("this account is disabled")]
    Disabled,
}

impl LoginError {
    pub fn code(&self) -> &'static str {
        match self {
            LoginError::EmailNotVerified => "email_not_verified",
            LoginError::RegistrationClosed => "registration_closed",
            LoginError::Disabled => "account_disabled",
        }
    }
}

pub fn email_domain(email: &str) -> Option<String> {
    email.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase())
}

/// Whether a new account may be created for this verified email, under the configured policy.
pub fn registration_allows(mode: &RegistrationMode, email: &str, has_invitation: bool, has_admin_grant: bool) -> bool {
    if has_admin_grant || has_invitation {
        return true;
    }
    match mode {
        RegistrationMode::Open => true,
        RegistrationMode::InviteOnly => false,
        RegistrationMode::Domains(domains) => email_domain(email).is_some_and(|d| domains.contains(&d)),
    }
}

/// Resolve a verified identity to an internal user, creating one when policy allows.
/// The provider subject is the identity key; email is profile data and never used to link
/// accounts.
pub async fn complete_login(db: &PgPool, config: &Config, identity: &VerifiedIdentity) -> anyhow::Result<Result<Uuid, LoginError>> {
    if !identity.email_verified {
        return Ok(Err(LoginError::EmailNotVerified));
    }
    let email = identity.email.trim().to_ascii_lowercase();
    let display_name = identity.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| email.split('@').next().unwrap_or(&email).to_string());
    let mut tx = db.begin().await?;

    let grant: Option<String> = sqlx::query_scalar("SELECT email FROM instance_admin_grants WHERE email = $1 AND consumed_at IS NULL FOR UPDATE")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;

    let existing: Option<(Uuid, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT u.id, u.disabled_at FROM identity_accounts i JOIN users u ON u.id = i.user_id WHERE i.provider = $1 AND i.subject = $2")
            .bind(&identity.provider)
            .bind(&identity.subject)
            .fetch_optional(&mut *tx)
            .await?;

    let user_id = match existing {
        Some((_, Some(_))) => return Ok(Err(LoginError::Disabled)),
        Some((user_id, None)) => {
            sqlx::query("UPDATE identity_accounts SET email = $3, last_login_at = now() WHERE provider = $1 AND subject = $2")
                .bind(&identity.provider)
                .bind(&identity.subject)
                .bind(&email)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE users SET email = $2, avatar_url = COALESCE($3, avatar_url), updated_at = now() WHERE id = $1")
                .bind(user_id)
                .bind(&email)
                .bind(&identity.picture)
                .execute(&mut *tx)
                .await?;
            user_id
        }
        None => {
            let has_invitation: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM invitations WHERE lower(email) = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now())",
            )
            .bind(&email)
            .fetch_one(&mut *tx)
            .await?;
            if !registration_allows(&config.registration, &email, has_invitation, grant.is_some()) {
                return Ok(Err(LoginError::RegistrationClosed));
            }
            let user_id: Uuid = sqlx::query_scalar("INSERT INTO users (email, display_name, avatar_url) VALUES ($1, $2, $3) RETURNING id")
                .bind(&email)
                .bind(display_name.chars().take(120).collect::<String>())
                .bind(&identity.picture)
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO identity_accounts (user_id, provider, subject, email) VALUES ($1, $2, $3, $4)")
                .bind(user_id)
                .bind(&identity.provider)
                .bind(&identity.subject)
                .bind(&email)
                .execute(&mut *tx)
                .await?;
            crate::audit::record_tx(
                &mut tx,
                None,
                Some(user_id),
                "user.registered",
                "user",
                Some(user_id),
                serde_json::json!({ "provider": identity.provider }),
            )
            .await?;
            user_id
        }
    };

    if grant.is_some() {
        sqlx::query("UPDATE users SET is_instance_admin = true WHERE id = $1").bind(user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE instance_admin_grants SET consumed_at = now() WHERE email = $1").bind(&email).execute(&mut *tx).await?;
        crate::audit::record_tx(&mut tx, None, Some(user_id), "instance_admin.granted", "user", Some(user_id), serde_json::json!({})).await?;
    }
    tx.commit().await?;
    Ok(Ok(user_id))
}

pub struct NewSession {
    pub token: String,
    pub csrf_token: String,
}

pub async fn create_session(db: &PgPool, config: &Config, user_id: Uuid, user_agent: Option<&str>, ip: Option<IpAddr>) -> anyhow::Result<NewSession> {
    let token = random_token();
    let csrf_token = random_token();
    // Restore the organization the user last worked in, if they are still a member.
    let active_org: Option<Uuid> = sqlx::query_scalar(
        "SELECT s.active_organization_id FROM sessions s
         JOIN organization_memberships m ON m.organization_id = s.active_organization_id AND m.user_id = s.user_id
         WHERE s.user_id = $1 ORDER BY s.created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .flatten();
    let active_org = match active_org {
        Some(id) => Some(id),
        None => {
            sqlx::query_scalar("SELECT organization_id FROM organization_memberships WHERE user_id = $1 ORDER BY created_at LIMIT 1")
                .bind(user_id)
                .fetch_optional(db)
                .await?
        }
    };
    let expires_at = Utc::now() + chrono::Duration::from_std(config.session_ttl)?;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, csrf_token, active_organization_id, user_agent, ip_address, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(hash_token(&token))
    .bind(user_id)
    .bind(&csrf_token)
    .bind(active_org)
    .bind(user_agent.map(|ua| ua.chars().take(300).collect::<String>()))
    .bind(ip.map(|ip| ip.to_string()))
    .bind(expires_at)
    .execute(db)
    .await?;
    Ok(NewSession { token, csrf_token })
}

pub fn session_cookie(config: &Config, token: String) -> Cookie<'static> {
    let mut cookie = Cookie::new(config.session_cookie_name(), token);
    cookie.set_http_only(true);
    cookie.set_secure(config.secure_cookies());
    cookie.set_same_site(SameSite::Lax);
    cookie.set_path("/");
    cookie.set_max_age(time::Duration::seconds(config.session_ttl.as_secs() as i64));
    cookie
}

pub fn expired_cookie(name: &'static str, path: &'static str) -> Cookie<'static> {
    let mut cookie = Cookie::new(name, "");
    cookie.set_path(path);
    cookie.set_max_age(time::Duration::ZERO);
    cookie
}

pub fn client_ip(parts: &Parts, config: &Config) -> Option<IpAddr> {
    if config.trust_proxy_headers {
        let forwarded =
            parts.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.split(',').next()).and_then(|v| v.trim().parse().ok());
        if forwarded.is_some() {
            return forwarded;
        }
    }
    parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|ci| ci.0.ip())
}

/// The client address, honouring X-Forwarded-For only when TRUST_PROXY_HEADERS is set.
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        Ok(ClientIp(client_ip(parts, &state.config).unwrap_or(IpAddr::from([127, 0, 0, 1]))))
    }
}

/// The signed-in user behind a request.
#[derive(Clone, Debug, Serialize)]
pub struct CurrentUser {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub discord_user_id: Option<String>,
    pub is_instance_admin: bool,
    pub active_organization_id: Option<Uuid>,
    #[serde(skip)]
    pub csrf_token: String,
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    session_id: Uuid,
    user_id: Uuid,
    csrf_token: String,
    active_organization_id: Option<Uuid>,
    last_seen_at: DateTime<Utc>,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    discord_user_id: Option<String>,
    is_instance_admin: bool,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(user) = parts.extensions.get::<CurrentUser>() {
            return Ok(user.clone());
        }
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar.get(state.config.session_cookie_name()).map(|c| c.value().to_string()).ok_or(AppError::Unauthenticated)?;
        let row: Option<SessionRow> = sqlx::query_as(
            "SELECT s.id AS session_id, s.user_id, s.csrf_token, s.active_organization_id, s.last_seen_at,
                    u.email, u.display_name, u.avatar_url, u.discord_user_id, u.is_instance_admin
             FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() AND u.disabled_at IS NULL",
        )
        .bind(hash_token(&token))
        .fetch_optional(&state.db)
        .await?;
        let row = row.ok_or(AppError::Unauthenticated)?;

        if !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS) {
            let given = parts.headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
            if !constant_time_eq(given.as_bytes(), row.csrf_token.as_bytes()) {
                return Err(AppError::Forbidden("missing or invalid CSRF token".into()));
            }
            let ip = client_ip(parts, &state.config).unwrap_or(IpAddr::from([127, 0, 0, 1]));
            if !state.write_limiter.check(ip, "write") {
                return Err(AppError::RateLimited);
            }
        }

        if Utc::now() - row.last_seen_at > chrono::Duration::minutes(5) {
            sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE id = $1").bind(row.session_id).execute(&state.db).await?;
        }

        let user = CurrentUser {
            user_id: row.user_id,
            session_id: row.session_id,
            email: row.email,
            display_name: row.display_name,
            avatar_url: row.avatar_url,
            discord_user_id: row.discord_user_id,
            is_instance_admin: row.is_instance_admin,
            active_organization_id: row.active_organization_id,
            csrf_token: row.csrf_token,
        };
        parts.extensions.insert(user.clone());
        Ok(user)
    }
}

/// The signed-in user acting inside their active organization, with their role there.
#[derive(Clone, Debug)]
pub struct OrgContext {
    pub user: CurrentUser,
    pub organization_id: Uuid,
    pub role: Role,
}

impl OrgContext {
    pub fn require(&self, permission: Permission) -> AppResult<()> {
        if self.role.allows(permission) {
            Ok(())
        } else {
            Err(AppError::forbidden())
        }
    }

    pub fn user_id(&self) -> Uuid {
        self.user.user_id
    }
}

impl FromRequestParts<AppState> for OrgContext {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        let organization_id = user.active_organization_id.ok_or_else(|| AppError::Forbidden("select or create an organization first".into()))?;
        let role: Option<String> = sqlx::query_scalar("SELECT role FROM organization_memberships WHERE organization_id = $1 AND user_id = $2")
            .bind(organization_id)
            .bind(user.user_id)
            .fetch_optional(&state.db)
            .await?;
        let role = role.and_then(|r| Role::parse(&r)).ok_or_else(|| AppError::Forbidden("you are no longer a member of this organization".into()))?;
        Ok(OrgContext { user, organization_id, role })
    }
}

/// Ensure every user id belongs to the organization. Used before referencing users in
/// schedules, teams and overrides so one tenant can never reference another tenant's people.
pub async fn ensure_members(tx: &mut Transaction<'_, Postgres>, organization_id: Uuid, user_ids: &[Uuid]) -> AppResult<()> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let unique: std::collections::HashSet<Uuid> = user_ids.iter().copied().collect();
    let ids: Vec<Uuid> = unique.into_iter().collect();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM organization_memberships WHERE organization_id = $1 AND user_id = ANY($2)")
        .bind(organization_id)
        .bind(&ids)
        .fetch_one(&mut **tx)
        .await?;
    if count as usize != ids.len() {
        return Err(AppError::validation("participants", "every person must be a member of this organization"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_policy() {
        let domains = RegistrationMode::Domains(vec!["example.com".into()]);
        assert!(registration_allows(&RegistrationMode::Open, "a@b.c", false, false));
        assert!(!registration_allows(&RegistrationMode::InviteOnly, "a@b.c", false, false));
        assert!(registration_allows(&RegistrationMode::InviteOnly, "a@b.c", true, false));
        assert!(registration_allows(&RegistrationMode::InviteOnly, "a@b.c", false, true));
        assert!(registration_allows(&domains, "a@Example.com", false, false));
        assert!(!registration_allows(&domains, "a@evil-example.com", false, false));
        assert!(!registration_allows(&domains, "a@example.com.evil.io", false, false));
    }
}
