//! Demo data for local development (`oncallkit seed-demo`). Refused in production.

use anyhow::{bail, Result};
use chrono::{Datelike, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    schedules::{
        engine::{resolve_local, RotationUnit},
        repo,
    },
};

pub async fn seed_demo(db: &PgPool, config: &Config, owner_email: &str) -> Result<Uuid> {
    if config.is_production() {
        bail!("seed-demo is for development only");
    }
    let owner_email = owner_email.trim().to_ascii_lowercase();
    let mut tx = db.begin().await?;

    let user = |email: String, name: &'static str| (email, name);
    let people = [
        user(owner_email.clone(), "You"),
        user("riky@demo.oncallkit.dev".into(), "Riky"),
        user("andi@demo.oncallkit.dev".into(), "Andi"),
        user("budi@demo.oncallkit.dev".into(), "Budi"),
        user("sari@demo.oncallkit.dev".into(), "Sari"),
    ];
    let mut ids = Vec::new();
    for (email, name) in &people {
        let existing: Option<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM identity_accounts WHERE provider = 'dev' AND subject = $1").bind(email).fetch_optional(&mut *tx).await?;
        let id = match existing {
            Some(id) => id,
            None => {
                let id: Uuid = sqlx::query_scalar("INSERT INTO users (email, display_name) VALUES ($1, $2) RETURNING id")
                    .bind(email)
                    .bind(name)
                    .fetch_one(&mut *tx)
                    .await?;
                sqlx::query("INSERT INTO identity_accounts (user_id, provider, subject, email) VALUES ($1, 'dev', $2, $2)")
                    .bind(id)
                    .bind(email)
                    .execute(&mut *tx)
                    .await?;
                id
            }
        };
        ids.push(id);
    }

    let org_id: Uuid = sqlx::query_scalar("INSERT INTO organizations (name, slug, timezone) VALUES ('Demo Platform', $1, 'Asia/Jakarta') RETURNING id")
        .bind(format!("demo-{}", &Uuid::new_v4().simple().to_string()[..8]))
        .fetch_one(&mut *tx)
        .await?;
    for (i, id) in ids.iter().enumerate() {
        sqlx::query("INSERT INTO organization_memberships (organization_id, user_id, role) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind(id)
            .bind(if i == 0 {
                "owner"
            } else if i == 1 {
                "scheduler"
            } else {
                "member"
            })
            .execute(&mut *tx)
            .await?;
    }
    let team_id: Uuid =
        sqlx::query_scalar("INSERT INTO teams (organization_id, name, description) VALUES ($1, 'DevOps', 'Infrastructure and deploys') RETURNING id")
            .bind(org_id)
            .fetch_one(&mut *tx)
            .await?;
    for id in &ids[1..] {
        sqlx::query("INSERT INTO team_memberships (team_id, user_id) VALUES ($1, $2)").bind(team_id).bind(id).execute(&mut *tx).await?;
    }

    // A weekly rotation that started last Tuesday at 09:00 Jakarta time.
    let tz: chrono_tz::Tz = "Asia/Jakarta".parse()?;
    let today = Utc::now().with_timezone(&tz).date_naive();
    let days_since_tuesday = (today.weekday().num_days_from_monday() + 7 - 1) % 7;
    let start = resolve_local(tz, (today - Duration::days(i64::from(days_since_tuesday) + 7)).and_hms_opt(9, 0, 0).unwrap());
    let schedule_id: Uuid = sqlx::query_scalar(
        "INSERT INTO schedules (organization_id, team_id, name, description, timezone, start_at, created_by)
         VALUES ($1, $2, 'DevOps On-call', 'Weekly hand-over every Tuesday at 09:00 WIB', 'Asia/Jakarta', $3, $4) RETURNING id",
    )
    .bind(org_id)
    .bind(team_id)
    .bind(start)
    .bind(ids[0])
    .fetch_one(&mut *tx)
    .await?;
    repo::insert_version(&mut tx, schedule_id, start, 7, RotationUnit::Days, &[ids[1], ids[2], ids[3]], &[ids[4], ids[1]], Some(ids[0])).await?;

    let daily_start = resolve_local(tz, (today - Duration::days(2)).and_hms_opt(9, 0, 0).unwrap());
    let daily_id: Uuid = sqlx::query_scalar(
        "INSERT INTO schedules (organization_id, team_id, name, description, timezone, start_at, end_at, created_by)
         VALUES ($1, $2, 'SRE Daily', 'Daily rotation, ends after 60 days', 'Asia/Jakarta', $3, $4, $5) RETURNING id",
    )
    .bind(org_id)
    .bind(team_id)
    .bind(daily_start)
    .bind(daily_start + Duration::days(60))
    .bind(ids[0])
    .fetch_one(&mut *tx)
    .await?;
    repo::insert_version(&mut tx, daily_id, daily_start, 1, RotationUnit::Days, &[ids[2], ids[3], ids[4]], &[], Some(ids[0])).await?;
    tx.commit().await?;
    Ok(org_id)
}
