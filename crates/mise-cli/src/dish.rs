//! `mise dish ...`: record decisions about how recipes group into dishes.

use anyhow::Result;
use clap::Subcommand;
use sqlx::PgPool;

#[derive(Subcommand)]
pub enum DishCommand {
    /// Load a decisions file (docs/curation.md), replacing the recorded decisions. Run group-dishes after.
    Apply { file: std::path::PathBuf },
}

pub async fn run(pool: &PgPool, cmd: DishCommand) -> Result<()> {
    match cmd {
        DishCommand::Apply { file } => {
            let stats = mise_ingest::dish_decisions::apply_file(pool, &file).await?;
            tracing::info!(?stats, "dish decisions recorded");
        }
    }
    Ok(())
}
