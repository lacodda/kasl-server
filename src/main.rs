mod app;
mod config;
mod model;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("kasl_server=info,tower_http=info")))
        .init();

    let config = config::Config::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;
    let migrator = sqlx::migrate!();
    // Worth a line: on a fresh install this is where the schema appears, and
    // on an upgrade it is the first thing to check when something looks off.
    let target = migrator.migrations.last().map(|m| m.version).unwrap_or_default();
    migrator.run(&pool).await.context("failed to apply database migrations")?;
    tracing::info!(version = target, "database schema is up to date");

    let listener = TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("failed to bind {}", config.addr))?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), addr = %config.addr, "kasl-server listening");

    axum::serve(listener, app::router(pool))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install the shutdown signal handler");
        return;
    }
    tracing::info!("shutting down");
}
