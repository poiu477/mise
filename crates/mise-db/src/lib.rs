//! Database access: connection pool and migrations.

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Migrations embedded from the workspace `migrations/` directory.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// A pool of up to 8 connections to the Postgres at `database_url` (`DATABASE_URL`: "postgres://host:5432/mise").
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(8)
        .connect(database_url)
        .await
        .context("connecting to Postgres")
}

/// Creates the database `database_url` names when it doesn't exist yet; answers whether it made it.
pub async fn create_if_missing(database_url: &str) -> anyhow::Result<bool> {
    use sqlx::migrate::MigrateDatabase;
    if sqlx::Postgres::database_exists(database_url)
        .await
        .context("asking Postgres whether the database exists")?
    {
        return Ok(false);
    }
    sqlx::Postgres::create_database(database_url)
        .await
        .context("creating the database")?;
    Ok(true)
}

/// Brings the database up to the newest migration; ones already applied are skipped, so it's safe on every start.
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    MIGRATOR.run(pool).await.context("running migrations")?;
    Ok(())
}
