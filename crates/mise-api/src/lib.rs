//! The mise REST API. One router backs the web app, kitchen mode and integrations; the OpenAPI document at
//! `/api/openapi.json` describes it.

pub mod auth;
pub mod dishes;
pub mod error;
pub mod extract;
pub mod media;
pub mod plan;
pub mod shopping;
pub mod variants;

use std::path::{Path, PathBuf};

use axum::{
    Json, Router, middleware,
    routing::{any, delete, get, patch, post, put},
};
use sqlx::PgPool;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use utoipa::OpenApi;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Where stored photos and their web-size copies live (MEDIA_ROOT).
    pub media_root: PathBuf,
    pub auth: std::sync::Arc<auth::Auth>,
}

#[derive(OpenApi)]
#[openapi(
    info(title = "mise", description = "Self-hosted recipe system: dishes, their versions, and any serving count."),
    paths(
        health, dishes::list, dishes::get, variants::get, media::get,
        auth::routes::config, auth::routes::setup, auth::routes::login, auth::routes::logout, auth::oidc::start, auth::oidc::callback,
        auth::routes::me, auth::routes::change_password, auth::routes::list_tokens, auth::routes::create_token, auth::routes::delete_token,
        auth::routes::list_users, auth::routes::create_user, auth::routes::update_user, auth::routes::list_households, auth::routes::create_household,
        plan::get, plan::add, plan::update, plan::remove,
        shopping::get, shopping::mark, shopping::add_extra, shopping::update_extra, shopping::remove_extra, shopping::add_staple, shopping::remove_staple,
        shopping::settings, shopping::update_settings, shopping::set_food_weights,
    ),
    tags(
        (name = "dishes", description = "Browse and search dishes"), (name = "media", description = "Photos"),
        (name = "plan", description = "The household's weekly plan"), (name = "shopping", description = "The shopping list worked out from the plan"),
        (name = "auth", description = "Signing in"), (name = "account", description = "The signed-in account"),
        (name = "admin", description = "Accounts and households (admins)"),
    ),
    modifiers(&Security)
)]
pub struct ApiDoc;

/// Documents the two ways to authenticate: the browser session cookie and a personal API token.
struct Security;

impl utoipa::Modify for Security {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme};
        let c = openapi.components.get_or_insert_with(Default::default);
        c.add_security_scheme("session", SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(auth::SESSION_COOKIE))));
        c.add_security_scheme("token", SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()));
        openapi.security = Some(vec![SecurityRequirement::new("session", Vec::<String>::new()), SecurityRequirement::new("token", Vec::<String>::new())]);
    }
}

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
        .route("/api/auth/config", get(auth::routes::config))
        .route("/api/auth/setup", post(auth::routes::setup))
        .route("/api/auth/login", post(auth::routes::login))
        .route("/api/auth/logout", post(auth::routes::logout))
        .route("/api/auth/oidc/start", get(auth::oidc::start))
        .route("/api/auth/oidc/callback", get(auth::oidc::callback))
        .route("/api/me", get(auth::routes::me))
        .route("/api/me/password", post(auth::routes::change_password))
        .route("/api/me/tokens", get(auth::routes::list_tokens).post(auth::routes::create_token))
        .route("/api/me/tokens/{id}", delete(auth::routes::delete_token))
        .route("/api/users", get(auth::routes::list_users).post(auth::routes::create_user))
        .route("/api/users/{id}", patch(auth::routes::update_user))
        .route("/api/households", get(auth::routes::list_households).post(auth::routes::create_household))
        .route("/api/openapi.json", get(openapi))
        .route("/api/dishes", get(dishes::list))
        .route("/api/dishes/{slug}", get(dishes::get))
        .route("/api/variants/{id}", get(variants::get))
        .route("/api/media/{id}/{size}", get(media::get))
        .route("/api/plan", get(plan::get))
        .route("/api/plan/entries", post(plan::add))
        .route("/api/plan/entries/{id}", patch(plan::update).delete(plan::remove))
        .route("/api/shopping", get(shopping::get))
        .route("/api/shopping/marks", put(shopping::mark))
        .route("/api/shopping/settings", get(shopping::settings).put(shopping::update_settings))
        .route("/api/shopping/food-weights", put(shopping::set_food_weights))
        .route("/api/shopping/extras", post(shopping::add_extra))
        .route("/api/shopping/extras/{id}", patch(shopping::update_extra).delete(shopping::remove_extra))
        .route("/api/pantry-staples", post(shopping::add_staple))
        .route("/api/pantry-staples/{food_id}", delete(shopping::remove_staple))
        .route("/api", any(|| async { error::ApiError::NotFound }))
        .route("/api/{*rest}", any(|| async { error::ApiError::NotFound }))
        .layer(middleware::from_fn_with_state(state.clone(), auth::require_auth))
        .with_state(state);
    if let Some(dir) = web_dir {
        app = app.fallback_service(web_app(dir));
    }
    app.layer(CompressionLayer::new()).layer(TraceLayer::new_for_http())
}

/// The built web app: its files, with any other path answered by index.html (status 200) so client-side
/// routes work on reload and the service worker can keep them for offline use.
fn web_app(dir: &Path) -> ServeDir<ServeFile> {
    ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))
}
