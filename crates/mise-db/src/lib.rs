//! Database access: connection pool and migrations.

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Migrations embedded from the workspace `migrations/` directory.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(8)
        .connect(database_url)
        .await
        .context("connecting to Postgres")
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    MIGRATOR.run(pool).await.context("running migrations")?;
    Ok(())
}
