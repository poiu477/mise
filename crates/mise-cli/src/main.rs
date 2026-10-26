//! `mise` command-line tool: database setup and data-core jobs.

mod food;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "mise", version, about = "mise recipe system: admin and data tools")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", global = true, hide_env_values = true)]
    database_url: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply database migrations.
    Migrate,
    /// Import a seed bundle directory (see docs/seed-bundle.md).
    ImportBundle {
        /// Directory containing manifest.json and variants.ndjson.
        dir: std::path::PathBuf,
    },
    /// Match ingredient lines to foods and propose the food tree.
    MatchFoods {
        /// First undo the previous matching run (keeps imported food knowledge).
        #[arg(long)]
        reset: bool,
    },
    /// Inspect and curate foods, brands and the food tree.
    #[command(subcommand)]
    Food(food::FoodCommand),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=error")))
        .init();

    let cli = Cli::parse();
    let database_url = cli
        .database_url
        .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is not set (see .env.example)"))?;

    let pool = mise_db::connect(&database_url).await?;
    match cli.command {
        Command::Migrate => {
            mise_db::migrate(&pool).await?;
            tracing::info!("migrations applied");
        }
        Command::ImportBundle { dir } => {
            let started = std::time::Instant::now();
            let stats = mise_ingest::bundle::import_bundle(&pool, &dir).await?;
            tracing::info!(?stats, elapsed_s = started.elapsed().as_secs(), "bundle imported");
            let foods = dir.join("foods.ndjson");
            if foods.exists() {
                let added = mise_ingest::foods::import_food_knowledge(&pool, &foods).await?;
                tracing::info!(added, "food knowledge imported");
            }
        }
        Command::MatchFoods { reset } => {
            let started = std::time::Instant::now();
            if reset {
                mise_ingest::foods::reset_matching(&pool).await?;
                tracing::info!("previous matching undone");
            }
            let stats = mise_ingest::foods::match_foods(&pool).await?;
            tracing::info!(?stats, elapsed_s = started.elapsed().as_secs(), "foods matched");
        }
        Command::Food(cmd) => food::run(&pool, cmd).await?,
    }
    Ok(())
}
