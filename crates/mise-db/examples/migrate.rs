//! Creates the database DATABASE_URL names when it doesn't exist, and brings it up to the newest migration. For
//! scripts/sqlx-prepare's scratch database: it needs only mise-db, so it runs before mise-api's checked queries can
//! compile against the schema.

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL must name the database to migrate"))?;
    if mise_db::create_if_missing(&url).await? {
        println!("created the database");
    }
    let pool = mise_db::connect(&url).await?;
    mise_db::migrate(&pool).await?;
    println!("migrated to the newest migration");
    Ok(())
}
