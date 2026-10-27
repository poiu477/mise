//! The mise REST API. One router backs the web app, kitchen mode and integrations; the OpenAPI document at
//! `/api/openapi.json` describes it.

pub mod dishes;
pub mod error;
pub mod media;
pub mod variants;

use std::path::PathBuf;

use axum::{Json, Router, routing::get};
use sqlx::PgPool;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
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
#[utoipa::path(get, path = "/api/health", responses((status = 200, body = Health)))]
pub async fn health() -> Json<Health> {
    Json(Health { status: "ok", version: env!("CARGO_PKG_VERSION") })
}

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/openapi.json", get(openapi))
        .route("/api/dishes", get(dishes::list))
        .route("/api/dishes/{slug}", get(dishes::get))
        .route("/api/variants/{id}", get(variants::get))
        .route("/api/media/{id}/{size}", get(media::get))
        .with_state(state)
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
