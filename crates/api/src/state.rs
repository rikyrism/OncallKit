use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::{
    auth::google::GoogleOidc,
    config::Config,
    crypto::SecretBox,
    integrations::discord::DiscordClient,
    observability::{Metrics, RateLimiter},
};

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub secrets: SecretBox,
    pub metrics: Metrics,
    pub discord: DiscordClient,
    pub google: Option<Arc<GoogleOidc>>,
    pub auth_limiter: Arc<RateLimiter>,
    pub write_limiter: Arc<RateLimiter>,
}

impl AppState {
    pub async fn connect(config: Config) -> Result<Self> {
        let db = PgPoolOptions::new()
            .max_connections(config.database_max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&config.database_url)
            .await
            .context("could not connect to PostgreSQL")?;
        let google = match &config.google {
            Some(g) => Some(Arc::new(GoogleOidc::discover(g, &config).await.context("Google OpenID Connect discovery failed")?)),
            None => None,
        };
        Ok(Self::with_pool(db, config, google))
    }

    pub fn with_pool(db: PgPool, config: Config, google: Option<Arc<GoogleOidc>>) -> Self {
        AppState {
            db,
            secrets: SecretBox::new(config.encryption_key),
            discord: DiscordClient::new(config.discord_base_override.clone()),
            metrics: Metrics::new(),
            google,
            auth_limiter: Arc::new(RateLimiter::new(30, Duration::from_secs(60))),
            write_limiter: Arc::new(RateLimiter::new(300, Duration::from_secs(60))),
            config: Arc::new(config),
        }
    }
}
