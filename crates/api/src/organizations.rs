//! Organizations, memberships and invitations.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit,
    auth::{CurrentUser, OrgContext},
    config::OrgCreation,
    crypto::{hash_token, random_token},
    error::{AppError, AppResult},
    rbac::{Permission, Role},
    schedules::repo as schedule_repo,
    state::AppState,
};

#[derive(Serialize, sqlx::FromRow)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub timezone: String,
    pub created_at: DateTime<Utc>,
}

pub fn validate_timezone(tz: &str) -> AppResult<chrono_tz::Tz> {
    tz.parse::<chrono_tz::Tz>().map_err(|_| AppError::validation("timezone", "use an IANA timezone such as Asia/Jakarta"))
}

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-').chars().take(48).collect::<String>();
    if slug.is_empty() {
        "org".into()
    } else {
        slug
    }
}

pub async fn list(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Vec<Organization>>> {
    let orgs = sqlx::query_as(
        "SELECT o.id, o.name, o.slug, o.timezone, o.created_at FROM organizations o
         JOIN organization_memberships m ON m.organization_id = o.id WHERE m.user_id = $1 ORDER BY o.name",
    )
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(orgs))
}

#[derive(Deserialize)]
pub struct CreateOrganization {
    name: String,
    timezone: Option<String>,
}

pub async fn create(State(state): State<AppState>, user: CurrentUser, Json(body): Json<CreateOrganization>) -> AppResult<(StatusCode, Json<Organization>)> {
    if state.config.org_creation == OrgCreation::InstanceAdmins && !user.is_instance_admin {
        return Err(AppError::Forbidden("only instance administrators can create organizations on this server".into()));
    }
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(AppError::validation("name", "use 1 to 120 characters"));
    }
    let timezone = body.timezone.unwrap_or_else(|| "UTC".into());
    validate_timezone(&timezone)?;

    let mut tx = state.db.begin().await?;
    let base = slugify(name);
    let mut slug = base.clone();
    for attempt in 1..20 {
        let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM organizations WHERE slug = $1)").bind(&slug).fetch_one(&mut *tx).await?;
        if !taken {
            break;
        }
        slug = format!("{base}-{}", attempt + 1);
    }
    let org: Organization =
        sqlx::query_as("INSERT INTO organizations (name, slug, timezone) VALUES ($1, $2, $3) RETURNING id, name, slug, timezone, created_at")
            .bind(name)
            .bind(&slug)
            .bind(&timezone)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("INSERT INTO organization_memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')")
        .bind(org.id)
        .bind(user.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE sessions SET active_organization_id = $1 WHERE id = $2").bind(org.id).bind(user.session_id).execute(&mut *tx).await?;
    audit::record_tx(&mut tx, Some(org.id), Some(user.user_id), "organization.created", "organization", Some(org.id), json!({ "name": name })).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(org)))
}

pub async fn get(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> AppResult<Json<Organization>> {
    let org = sqlx::query_as(
        "SELECT o.id, o.name, o.slug, o.timezone, o.created_at FROM organizations o
         JOIN organization_memberships m ON m.organization_id = o.id WHERE o.id = $1 AND m.user_id = $2",
    )
    .bind(id)
    .bind(user.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(org))
}

pub async fn activate(State(state): State<AppState>, user: CurrentUser, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let member: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM organization_memberships WHERE organization_id = $1 AND user_id = $2)")
        .bind(id)
        .bind(user.user_id)
        .fetch_one(&state.db)
        .await?;
    if !member {
        return Err(AppError::NotFound);
    }
    sqlx::query("UPDATE sessions SET active_organization_id = $1 WHERE id = $2").bind(id).bind(user.session_id).execute(&state.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct UpdateOrganization {
    name: Option<String>,
    timezone: Option<String>,
}

pub async fn update_current(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<UpdateOrganization>) -> AppResult<Json<Organization>> {
    ctx.require(Permission::ManageOrganization)?;
    if let Some(name) = &body.name {
        if name.trim().is_empty() || name.trim().chars().count() > 120 {
            return Err(AppError::validation("name", "use 1 to 120 characters"));
        }
    }
    if let Some(tz) = &body.timezone {
        validate_timezone(tz)?;
    }
    let mut tx = state.db.begin().await?;
    let org: Organization = sqlx::query_as(
        "UPDATE organizations SET name = COALESCE($2, name), timezone = COALESCE($3, timezone), updated_at = now()
         WHERE id = $1 RETURNING id, name, slug, timezone, created_at",
    )
    .bind(ctx.organization_id)
    .bind(body.name.as_deref().map(str::trim))
    .bind(&body.timezone)
    .fetch_one(&mut *tx)
    .await?;
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "organization.updated", "organization", Some(org.id), json!({})).await?;
    tx.commit().await?;
    Ok(Json(org))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Member {
    user_id: Uuid,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    role: String,
    joined_at: DateTime<Utc>,
    team_ids: Vec<Uuid>,
}

pub async fn list_members(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Vec<Member>>> {
    let members = sqlx::query_as(
        "SELECT u.id AS user_id, u.email, u.display_name, u.avatar_url, m.role, m.created_at AS joined_at,
                COALESCE(array_agg(tm.team_id) FILTER (WHERE tm.team_id IS NOT NULL), '{}') AS team_ids
         FROM organization_memberships m
         JOIN users u ON u.id = m.user_id
         LEFT JOIN team_memberships tm ON tm.user_id = u.id AND tm.team_id IN (SELECT id FROM teams WHERE organization_id = $1)
         WHERE m.organization_id = $1
         GROUP BY u.id, m.role, m.created_at ORDER BY u.display_name",
    )
    .bind(ctx.organization_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(members))
}

#[derive(Deserialize)]
pub struct UpdateMember {
    role: String,
}

async fn member_role(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, org: Uuid, user: Uuid) -> AppResult<Role> {
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM organization_memberships WHERE organization_id = $1 AND user_id = $2 FOR UPDATE")
        .bind(org)
        .bind(user)
        .fetch_optional(&mut **tx)
        .await?;
    role.and_then(|r| Role::parse(&r)).ok_or(AppError::NotFound)
}

pub async fn update_member(State(state): State<AppState>, ctx: OrgContext, Path(user_id): Path<Uuid>, Json(body): Json<UpdateMember>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageMembers)?;
    let new_role = Role::parse(&body.role).ok_or_else(|| AppError::validation("role", "unknown role"))?;
    if new_role == Role::Owner {
        return Err(AppError::validation("role", "use ownership transfer to make someone an owner"));
    }
    let mut tx = state.db.begin().await?;
    let current = member_role(&mut tx, ctx.organization_id, user_id).await?;
    if !ctx.role.can_assign(current) || !ctx.role.can_assign(new_role) {
        return Err(AppError::forbidden());
    }
    if current == Role::Owner {
        return Err(AppError::Conflict("transfer ownership before changing the owner's role".into()));
    }
    sqlx::query("UPDATE organization_memberships SET role = $3 WHERE organization_id = $1 AND user_id = $2")
        .bind(ctx.organization_id)
        .bind(user_id)
        .bind(new_role.as_str())
        .execute(&mut *tx)
        .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "member.role_changed",
        "user",
        Some(user_id),
        json!({ "from": current.as_str(), "to": new_role.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_member(State(state): State<AppState>, ctx: OrgContext, Path(user_id): Path<Uuid>) -> AppResult<StatusCode> {
    let leaving_self = user_id == ctx.user_id();
    if !leaving_self {
        ctx.require(Permission::ManageMembers)?;
    }
    let mut tx = state.db.begin().await?;
    let current = member_role(&mut tx, ctx.organization_id, user_id).await?;
    if current == Role::Owner {
        return Err(AppError::Conflict("the owner cannot be removed; transfer ownership first".into()));
    }
    if !leaving_self && !ctx.role.can_assign(current) {
        return Err(AppError::forbidden());
    }
    // Take the person out of every rotation from now on, keeping their past shifts intact.
    schedule_repo::remove_user_from_rotations(&mut tx, ctx.organization_id, user_id, ctx.user_id()).await?;
    sqlx::query("DELETE FROM team_memberships WHERE user_id = $2 AND team_id IN (SELECT id FROM teams WHERE organization_id = $1)")
        .bind(ctx.organization_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE shift_swaps SET status = 'canceled', decided_at = now(), decided_by = $3
         WHERE organization_id = $1 AND status = 'pending' AND (requester_id = $2 OR target_user_id = $2)",
    )
    .bind(ctx.organization_id)
    .bind(user_id)
    .bind(ctx.user_id())
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM calendar_feeds WHERE organization_id = $1 AND user_id = $2").bind(ctx.organization_id).bind(user_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM organization_memberships WHERE organization_id = $1 AND user_id = $2")
        .bind(ctx.organization_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE sessions SET active_organization_id = NULL WHERE user_id = $1 AND active_organization_id = $2")
        .bind(user_id)
        .bind(ctx.organization_id)
        .execute(&mut *tx)
        .await?;
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "member.removed", "user", Some(user_id), json!({ "role": current.as_str() }))
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TransferOwnership {
    user_id: Uuid,
}

pub async fn transfer_ownership(State(state): State<AppState>, ctx: OrgContext, Json(body): Json<TransferOwnership>) -> AppResult<StatusCode> {
    ctx.require(Permission::TransferOwnership)?;
    if body.user_id == ctx.user_id() {
        return Err(AppError::validation("user_id", "you already own this organization"));
    }
    let mut tx = state.db.begin().await?;
    member_role(&mut tx, ctx.organization_id, body.user_id).await?;
    sqlx::query("UPDATE organization_memberships SET role = 'owner' WHERE organization_id = $1 AND user_id = $2")
        .bind(ctx.organization_id)
        .bind(body.user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE organization_memberships SET role = 'admin' WHERE organization_id = $1 AND user_id = $2")
        .bind(ctx.organization_id)
        .bind(ctx.user_id())
        .execute(&mut *tx)
        .await?;
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "organization.ownership_transferred", "user", Some(body.user_id), json!({}))
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Invitations ----

#[derive(Serialize, sqlx::FromRow)]
pub struct Invitation {
    id: Uuid,
    email: String,
    role: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    accepted_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    invited_by_name: Option<String>,
}

pub async fn list_invitations(State(state): State<AppState>, ctx: OrgContext) -> AppResult<Json<Vec<Invitation>>> {
    ctx.require(Permission::ManageMembers)?;
    let items = sqlx::query_as(
        "SELECT i.id, i.email, i.role, i.created_at, i.expires_at, i.accepted_at, i.revoked_at, u.display_name AS invited_by_name
         FROM invitations i LEFT JOIN users u ON u.id = i.invited_by
         WHERE i.organization_id = $1 ORDER BY i.created_at DESC LIMIT 200",
    )
    .bind(ctx.organization_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(items))
}

#[derive(Deserialize)]
pub struct CreateInvitation {
    email: String,
    role: String,
}

#[derive(Serialize)]
pub struct CreatedInvitation {
    id: Uuid,
    email: String,
    role: String,
    expires_at: DateTime<Utc>,
    /// The only time the link is shown. Share it with the invitee over a trusted channel.
    invite_url: String,
}

pub async fn create_invitation(
    State(state): State<AppState>,
    ctx: OrgContext,
    Json(body): Json<CreateInvitation>,
) -> AppResult<(StatusCode, Json<CreatedInvitation>)> {
    ctx.require(Permission::ManageMembers)?;
    let email = body.email.trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() > 254 || email.contains(char::is_whitespace) {
        return Err(AppError::validation("email", "enter a valid email address"));
    }
    let role = Role::parse(&body.role).filter(|r| *r != Role::Owner).ok_or_else(|| AppError::validation("role", "choose admin, scheduler or member"))?;
    if !ctx.role.can_assign(role) {
        return Err(AppError::forbidden());
    }
    let mut tx = state.db.begin().await?;
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM organization_memberships m JOIN users u ON u.id = m.user_id WHERE m.organization_id = $1 AND lower(u.email) = $2)",
    )
    .bind(ctx.organization_id)
    .bind(&email)
    .fetch_one(&mut *tx)
    .await?;
    if already {
        return Err(AppError::Conflict("this person is already a member".into()));
    }
    // A new invitation replaces any earlier open one for the same address.
    sqlx::query("UPDATE invitations SET revoked_at = now() WHERE organization_id = $1 AND lower(email) = $2 AND accepted_at IS NULL AND revoked_at IS NULL")
        .bind(ctx.organization_id)
        .bind(&email)
        .execute(&mut *tx)
        .await?;
    let token = random_token();
    let (id, expires_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO invitations (organization_id, email, role, token_hash, invited_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + interval '7 days') RETURNING id, expires_at",
    )
    .bind(ctx.organization_id)
    .bind(&email)
    .bind(role.as_str())
    .bind(hash_token(&token))
    .bind(ctx.user_id())
    .fetch_one(&mut *tx)
    .await?;
    audit::record_tx(
        &mut tx,
        Some(ctx.organization_id),
        Some(ctx.user_id()),
        "invitation.created",
        "invitation",
        Some(id),
        json!({ "email": email, "role": role.as_str() }),
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvitation { id, email, role: role.as_str().into(), expires_at, invite_url: state.config.app_url(&format!("/invite/{token}")) }),
    ))
}

pub async fn revoke_invitation(State(state): State<AppState>, ctx: OrgContext, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    ctx.require(Permission::ManageMembers)?;
    let mut tx = state.db.begin().await?;
    let result = sqlx::query("UPDATE invitations SET revoked_at = now() WHERE id = $1 AND organization_id = $2 AND accepted_at IS NULL AND revoked_at IS NULL")
        .bind(id)
        .bind(ctx.organization_id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit::record_tx(&mut tx, Some(ctx.organization_id), Some(ctx.user_id()), "invitation.revoked", "invitation", Some(id), json!({})).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct PendingInvitation {
    id: Uuid,
    organization_id: Uuid,
    organization_name: String,
    role: String,
    expires_at: DateTime<Utc>,
}

/// Open invitations addressed to the signed-in user's verified email.
pub async fn my_invitations(State(state): State<AppState>, user: CurrentUser) -> AppResult<Json<Vec<PendingInvitation>>> {
    let items = sqlx::query_as(
        "SELECT i.id, i.organization_id, o.name AS organization_name, i.role, i.expires_at
         FROM invitations i JOIN organizations o ON o.id = i.organization_id
         WHERE lower(i.email) = lower($1) AND i.accepted_at IS NULL AND i.revoked_at IS NULL AND i.expires_at > now()
           AND NOT EXISTS (SELECT 1 FROM organization_memberships m WHERE m.organization_id = i.organization_id AND m.user_id = $2)
         ORDER BY i.created_at DESC",
    )
    .bind(&user.email)
    .bind(user.user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(items))
}

#[derive(Deserialize)]
pub struct AcceptInvitation {
    /// The token from an invitation link, or
    token: Option<String>,
    /// the id of one of the user's own pending invitations.
    id: Option<Uuid>,
}

/// Accept an invitation. The invitation's email must match the signed-in user's verified email,
/// so a leaked link cannot be used by someone else.
pub async fn accept_invitation(State(state): State<AppState>, user: CurrentUser, Json(body): Json<AcceptInvitation>) -> AppResult<Json<serde_json::Value>> {
    let mut tx = state.db.begin().await?;
    let row: Option<(Uuid, Uuid, String, String)> = match (&body.token, body.id) {
        (Some(token), _) => {
            sqlx::query_as(
                "SELECT id, organization_id, email, role FROM invitations
             WHERE token_hash = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now() FOR UPDATE",
            )
            .bind(hash_token(token))
            .fetch_optional(&mut *tx)
            .await?
        }
        (None, Some(id)) => {
            sqlx::query_as(
                "SELECT id, organization_id, email, role FROM invitations
             WHERE id = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now() FOR UPDATE",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
        }
        (None, None) => return Err(AppError::BadRequest("token or id is required".into())),
    };
    let (invitation_id, organization_id, email, role) =
        row.ok_or_else(|| AppError::BadRequest("this invitation is invalid, expired or already used".into()))?;
    if !email.eq_ignore_ascii_case(&user.email) {
        return Err(AppError::Forbidden(format!("this invitation was sent to a different email address than {}", user.email)));
    }
    sqlx::query("INSERT INTO organization_memberships (organization_id, user_id, role) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
        .bind(organization_id)
        .bind(user.user_id)
        .bind(&role)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE invitations SET accepted_at = now(), accepted_by = $2 WHERE id = $1").bind(invitation_id).bind(user.user_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE sessions SET active_organization_id = $1 WHERE id = $2").bind(organization_id).bind(user.session_id).execute(&mut *tx).await?;
    audit::record_tx(&mut tx, Some(organization_id), Some(user.user_id), "invitation.accepted", "invitation", Some(invitation_id), json!({ "role": role }))
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "organization_id": organization_id })))
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Acme Platform Team"), "acme-platform-team");
        assert_eq!(slugify("  ---  "), "org");
        assert_eq!(slugify("SRE / DevOps!"), "sre-devops");
    }
}
