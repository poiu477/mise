//! The mise REST API. One router backs the web app, kitchen mode and integrations; the OpenAPI document at
//! `/api/openapi.json` describes it.

pub mod dishes;
pub mod error;
pub mod media;
pub mod variants;

use std::path::{Path, PathBuf};

use axum::{
    Json, Router,
    routing::{any, get},
};
use sqlx::PgPool;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use utoipa::OpenApi;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Where stored photos and their web-size copies live (MEDIA_ROOT).
    pub media_root: PathBuf,
}

#[derive(OpenApi)]
#[openapi(
    info(title = "mise", description = "Self-hosted recipe system: dishes, their versions, and any serving count."),
    paths(health, dishes::list, dishes::get, variants::get, media::get),
    tags((name = "dishes", description = "Browse and search dishes"), (name = "media", description = "Photos"))
)]
pub struct ApiDoc;

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

/// Liveness and version.
#[utoipa::path(get, path = "/api/health", operation_id = "health", responses((status = 200, body = Health)))]
pub async fn health() -> Json<Health> {
    Json(Health { status: "ok", version: env!("CARGO_PKG_VERSION") })
}

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// The API, and the built web app in `web_dir` for every other path when given.
pub fn router(state: AppState, web_dir: Option<&Path>) -> Router {
    let mut app = Router::new()
        .route("/api/health", get(health))
        .route("/api/openapi.json", get(openapi))
        .route("/api/dishes", get(dishes::list))
        .route("/api/dishes/{slug}", get(dishes::get))
        .route("/api/variants/{id}", get(variants::get))
        .route("/api/media/{id}/{size}", get(media::get))
        .route("/api/{*rest}", any(|| async { error::ApiError::NotFound }))
        .with_state(state);
    if let Some(dir) = web_dir {
        app = app.fallback_service(web_app(dir));
    }
    app.layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

/// The built web app: its files, with any other path answered by index.html so client-side routes work on
/// reload.
fn web_app(dir: &Path) -> ServeDir<tower_http::set_status::SetStatus<ServeFile>> {
    ServeDir::new(dir).not_found_service(ServeFile::new(dir.join("index.html")))
}
