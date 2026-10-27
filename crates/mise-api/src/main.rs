//! `mise-server`: the API (and, when built, the web app) on one port.
//!
//! Settings come from the environment (see .env.example): DATABASE_URL, MEDIA_ROOT, MISE_BIND, MISE_WEB_DIR
//! (the built web app, default ./web/dist when present) and MISE_MIGRATE (run migrations at start, default on).
//!
//! `mise-server openapi` prints the OpenAPI document and exits, without a database (the web app's types are
//! generated from it).

use std::path::PathBuf;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("openapi") {
        println!("{}", <mise_api::ApiDoc as utoipa::OpenApi>::openapi().to_pretty_json()?);
        return Ok(());
    }
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=error,tower_http=info")))
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set (see .env.example)")?;
    let media_root = PathBuf::from(std::env::var("MEDIA_ROOT").unwrap_or_else(|_| "./data/media".into()));
    let bind = std::env::var("MISE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let migrate = std::env::var("MISE_MIGRATE").map_or(true, |v| v != "0" && v != "false");
    let web_dir = std::env::var("MISE_WEB_DIR").map(PathBuf::from).ok().or_else(|| {
        let default = PathBuf::from("web/dist");
        default.join("index.html").exists().then_some(default)
    });

    let pool = mise_db::connect(&database_url).await?;
    if migrate {
        mise_db::migrate(&pool).await?;
    }
    let app = mise_api::router(mise_api::AppState { pool, media_root: media_root.clone() }, web_dir.as_deref());
    if let Some(dir) = &web_dir {
        tracing::info!(web_dir = %dir.display(), "serving the web app");
    }
    let listener = tokio::net::TcpListener::bind(&bind).await.with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, media_root = %media_root.display(), "mise-server listening");
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}
