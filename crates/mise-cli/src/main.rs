//! `mise` command-line tool: database setup and data-core jobs.

mod food;
mod gate;

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
    /// Group variants into dishes (re-runnable; leaves dishes and placements made by a person alone).
    GroupDishes,
    /// Derive each dish's options (protein, portion, version) from its variants. Re-runnable.
    DeriveOptions,
    /// Delete imported variants: one content type (e.g. ready meals), or whatever a re-exported bundle no longer
    /// contains. Re-run group-dishes and derive-options after.
    Prune {
        #[arg(long, value_parser = ["ready_meal", "product", "add_on"], required_unless_present = "missing_from", conflicts_with = "missing_from")]
        content_type: Option<String>,
        /// Seed bundle directory: remove this source's variants that are not in it.
        #[arg(long)]
        missing_from: Option<std::path::PathBuf>,
    },
    /// Store referenced photos under MEDIA_ROOT, link them to variants, steps and foods, and make web copies.
    /// Resumable: a re-run only reads files and makes copies it has not done yet.
    IngestMedia {
        /// Archive root the media paths are relative to (the bundle's `media_root`, e.g. D:/Recipes).
        #[arg(long, required_unless_present = "derivatives_only")]
        from: Option<std::path::PathBuf>,
        /// Where media is stored. Hard links are used when this is on the same volume as the archive.
        #[arg(long, env = "MEDIA_ROOT", default_value = "./data/media")]
        media_root: std::path::PathBuf,
        /// Read at most this many new files, and make web copies for at most this many media (trial runs).
        #[arg(long)]
        limit: Option<usize>,
        /// Store and link only; make web copies in a later run.
        #[arg(long)]
        skip_derivatives: bool,
        /// Only make missing web copies of media already stored.
        #[arg(long, conflicts_with = "skip_derivatives")]
        derivatives_only: bool,
        /// Worker threads for hashing and resizing (default: all cores).
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Inspect and curate foods, brands and the food tree.
    #[command(subcommand)]
    Food(food::FoodCommand),
    /// Write the phase 1 gate report: a stratified sample of dishes to check grouping by hand (read-only).
    GateReport {
        /// Number of dishes to sample.
        #[arg(long, default_value_t = 500)]
        sample: usize,
        /// HTML file to write.
        #[arg(long, default_value = "reports/gate-500.html")]
        out: std::path::PathBuf,
        /// Seed for the sample: the same seed on the same data picks the same dishes.
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
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
        Command::GroupDishes => {
            let started = std::time::Instant::now();
            let stats = mise_ingest::dishes::group_dishes(&pool).await?;
            tracing::info!(?stats, elapsed_s = started.elapsed().as_secs(), "dishes grouped");
        }
        Command::DeriveOptions => {
            let started = std::time::Instant::now();
            let stats = mise_ingest::options::derive_options(&pool).await?;
            tracing::info!(?stats, elapsed_s = started.elapsed().as_secs(), "options derived");
        }
        Command::Prune { content_type, missing_from } => {
            let stats = match (content_type, missing_from) {
                (Some(ct), _) => mise_ingest::prune::prune_content_type(&pool, &ct).await?,
                (None, Some(dir)) => mise_ingest::prune::prune_missing_from(&pool, &dir).await?,
                (None, None) => unreachable!("clap requires one of them"),
            };
            tracing::info!(?stats, "pruned");
        }
        Command::IngestMedia { from, media_root, limit, skip_derivatives, derivatives_only, jobs } => {
            let opts = mise_ingest::media::MediaOptions {
                archive_root: from,
                media_root,
                limit,
                jobs: jobs.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get())),
            };
            if !derivatives_only {
                let started = std::time::Instant::now();
                let stats = mise_ingest::media::store_and_link(&pool, &opts).await?;
                tracing::info!(?stats, elapsed_s = started.elapsed().as_secs(), "media stored and linked");
            }
            if !skip_derivatives {
                let started = std::time::Instant::now();
                let stats = mise_ingest::media::make_derivatives(&pool, &opts).await?;
                tracing::info!(?stats, elapsed_s = started.elapsed().as_secs(), "web copies made");
            }
        }
        Command::Food(cmd) => food::run(&pool, cmd).await?,
        Command::GateReport { sample, out, seed } => {
            let report = gate::write_report(&pool, sample, seed, &out).await?;
            let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            for s in &report.strata {
                tracing::info!(stratum = s.key, in_data = s.population, sampled = s.sampled, "stratum");
            }
            tracing::info!(dishes = report.cards.len(), seed, kib = bytes / 1024, out = %out.display(), "gate report written");
        }
    }
    Ok(())
}
