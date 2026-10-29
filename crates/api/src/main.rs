use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use oncallkit::{config::Config, routes, seed, state::AppState, worker, MIGRATOR};
use tokio::sync::watch;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(name = "oncallkit", version, about = "OncallKit on-call scheduling server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP API (and the worker, unless RUN_WORKER=false). Applies migrations first.
    Serve,
    /// Run only the background worker.
    Worker {
        /// Run a single cycle and exit.
        #[arg(long)]
        once: bool,
    },
    /// Apply database migrations and exit.
    Migrate,
    /// Pre-authorise an email as instance administrator. The grant is applied the first time
    /// that person signs in with a verified email, and lets them sign in even when registration
    /// is closed and create organizations.
    BootstrapAdmin {
        #[arg(long)]
        email: String,
    },
    /// Create a demo organization with sample people and schedules (development only).
    SeedDemo {
        /// The email you will sign in with; you become the demo organization's owner.
        #[arg(long)]
        owner_email: String,
    },
}

fn init_tracing(json: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    let registry = tracing_subscriber::registry().with(filter);
    if json {
        registry.with(fmt::layer().json().flatten_event(true).with_current_span(true)).init();
    } else {
        registry.with(fmt::layer().compact()).init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    let config = Config::from_env()?;
    init_tracing(config.log_json);

    match cli.command.unwrap_or(Command::Serve) {
        Command::Migrate => {
            let db = sqlx::PgPool::connect(&config.database_url).await.context("could not connect to PostgreSQL")?;
            MIGRATOR.run(&db).await?;
            tracing::info!("migrations applied");
        }
        Command::BootstrapAdmin { email } => {
            let db = sqlx::PgPool::connect(&config.database_url).await?;
            MIGRATOR.run(&db).await?;
            let email = email.trim().to_ascii_lowercase();
            let updated = sqlx::query("UPDATE users SET is_instance_admin = true WHERE id IN (SELECT user_id FROM identity_accounts WHERE lower(email) = $1)")
                .bind(&email)
                .execute(&db)
                .await?
                .rows_affected();
            if updated > 0 {
                println!("{email} is now an instance administrator.");
            } else {
                sqlx::query("INSERT INTO instance_admin_grants (email) VALUES ($1) ON CONFLICT (email) DO UPDATE SET consumed_at = NULL")
                    .bind(&email)
                    .execute(&db)
                    .await?;
                println!("{email} will become an instance administrator the first time they sign in with that verified email.");
            }
        }
        Command::SeedDemo { owner_email } => {
            let db = sqlx::PgPool::connect(&config.database_url).await?;
            MIGRATOR.run(&db).await?;
            let org = seed::seed_demo(&db, &config, &owner_email).await?;
            println!("Demo organization {org} created. Sign in as {owner_email} (with AUTH_DEV_LOGIN=true or Google) to see it.");
        }
        Command::Worker { once } => {
            let state = AppState::connect(config).await?;
            MIGRATOR.run(&state.db).await?;
            if once {
                worker::tick(&state, true).await?;
            } else {
                let (tx, rx) = watch::channel(false);
                let handle = tokio::spawn(worker::run(state, rx));
                shutdown_signal().await;
                let _ = tx.send(true);
                let _ = handle.await;
            }
        }
        Command::Serve => {
            if config.dev_login {
                tracing::warn!(
                    "AUTH_DEV_LOGIN is enabled: anyone who can reach this server can sign in as any email. Never use this outside local development."
                );
            }
            let bind = config.bind_addr;
            let run_worker = config.run_worker;
            let state = AppState::connect(config).await?;
            MIGRATOR.run(&state.db).await.context("database migration failed")?;
            let (tx, rx) = watch::channel(false);
            let worker_handle = run_worker.then(|| tokio::spawn(worker::run(state.clone(), rx)));
            let app = routes::router(state);
            let listener = tokio::net::TcpListener::bind(bind).await?;
            tracing::info!(%bind, "OncallKit API listening");
            axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).with_graceful_shutdown(shutdown_signal()).await?;
            let _ = tx.send(true);
            if let Some(h) = worker_handle {
                let _ = h.await;
            }
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
