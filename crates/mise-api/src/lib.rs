//! The mise REST API, served under `/api/v1`. One router backs the web app, kitchen mode and integrations; the
//! OpenAPI document at `/api/v1/openapi.json` describes it. The old `/api` addresses answer too, for one more
//! release, marked deprecated.

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
    Json, Router,
    extract::Request,
    http::{HeaderName, HeaderValue, Uri, header},
    middleware::{self, Next},
    response::Response,
    routing::{any, delete, get, patch, post, put},
};
use error::ApiError;
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
        use utoipa::openapi::security::{
            ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme,
        };
        let c = openapi.components.get_or_insert_with(Default::default);
        c.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(auth::SESSION_COOKIE))),
        );
        c.add_security_scheme(
            "token",
            SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
        );
        openapi.security = Some(vec![
            SecurityRequirement::new("session", Vec::<String>::new()),
            SecurityRequirement::new("token", Vec::<String>::new()),
        ]);
    }
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

/// Liveness and version. `/api/health` answers too, for good: proxies and monitors check it.
#[utoipa::path(get, path = "/api/v1/health", operation_id = "health", security(()),
    responses(
        (status = 200, description = "The server is up", body = Health)
    )
)]
pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// Where the API is served: `/api/v1/dishes`, `/api/v1/openapi.json` and so on.
pub const API: &str = "/api/v1";

/// Where the API was served before `/api/v1`. Its addresses answer as before, marked deprecated, for one more
/// release (docs/deploy.md says which), so a phone still running the web app it cached keeps working.
pub const OLD_API: &str = "/api";

/// Addresses under `/api` that stay as they are, never deprecated: the health check, which proxies and monitors
/// call, and single sign-on's browser redirects, which a sign-in provider has on record.
pub const PERMANENT: &[&str] = &[
    "/api/health",
    "/api/auth/oidc/start",
    "/api/auth/oidc/callback",
];

/// When the old addresses were deprecated, as RFC 9745 writes it: a Structured Fields date, `@` and a Unix time
/// (the drafts' `true` isn't in the RFC). 2026-11-10 is the day `/api/v1` was decided and built: a date in the past
/// tells a client the old address is deprecated already. The release that drops it is in docs/deploy.md.
pub const DEPRECATED_SINCE: &str = "@1794268800";

/// The part of `path` after the API's address, new or old: "/dishes" for "/api/v1/dishes" and "/api/dishes",
/// "" for "/api"; None outside the API (the web app).
pub fn api_path(path: &str) -> Option<&str> {
    [API, OLD_API].into_iter().find_map(|prefix| {
        path.strip_prefix(prefix)
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// The `/api/v1` address that replaces an old `/api` one, query included; None for every other address.
fn successor(uri: &Uri) -> Option<String> {
    let path = uri.path();
    let rest = path
        .strip_prefix(OLD_API)
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))?;
    if rest == "/v1" || rest.starts_with("/v1/") || PERMANENT.contains(&path) {
        return None;
    }
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    Some(format!("{API}{rest}{query}"))
}

/// Marks every answer from an old `/api` address deprecated (RFC 9745), with a link to the address that replaces
/// it (`rel="successor-version"`, RFC 5829). The answer is otherwise exactly the new address's.
async fn mark_deprecated(req: Request, next: Next) -> Response {
    let successor = successor(req.uri());
    let mut res = next.run(req).await;
    if let Some(link) = successor
        .and_then(|s| HeaderValue::from_str(&format!("<{s}>; rel=\"successor-version\"")).ok())
    {
        let headers = res.headers_mut();
        headers.insert(
            HeaderName::from_static("deprecation"),
            HeaderValue::from_static(DEPRECATED_SINCE),
        );
        headers.insert(header::LINK, link);
    }
    res
}

/// Every API route, relative to the API's address. Single sign-on's redirects aren't here: they keep their
/// addresses (see `PERMANENT`).
fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/auth/config", get(auth::routes::config))
        .route("/auth/setup", post(auth::routes::setup))
        .route("/auth/login", post(auth::routes::login))
        .route("/auth/logout", post(auth::routes::logout))
        .route("/me", get(auth::routes::me))
        .route("/me/password", post(auth::routes::change_password))
        .route(
            "/me/tokens",
            get(auth::routes::list_tokens).post(auth::routes::create_token),
        )
        .route("/me/tokens/{id}", delete(auth::routes::delete_token))
        .route(
            "/users",
            get(auth::routes::list_users).post(auth::routes::create_user),
        )
        .route("/users/{id}", patch(auth::routes::update_user))
        .route(
            "/households",
            get(auth::routes::list_households).post(auth::routes::create_household),
        )
        .route("/openapi.json", get(openapi))
        .route("/dishes", get(dishes::list))
        .route("/dishes/{slug}", get(dishes::get))
        .route("/variants/{id}", get(variants::get))
        .route("/media/{id}/{size}", get(media::get))
        .route("/plan", get(plan::get))
        .route("/plan/entries", post(plan::add))
        .route(
            "/plan/entries/{id}",
            patch(plan::update).delete(plan::remove),
        )
        .route("/shopping", get(shopping::get))
        .route("/shopping/marks", put(shopping::mark))
        .route(
            "/shopping/settings",
            get(shopping::settings).put(shopping::update_settings),
        )
        .route("/shopping/food-weights", put(shopping::set_food_weights))
        .route("/shopping/extras", post(shopping::add_extra))
        .route(
            "/shopping/extras/{id}",
            patch(shopping::update_extra).delete(shopping::remove_extra),
        )
        .route("/pantry-staples", post(shopping::add_staple))
        .route("/pantry-staples/{food_id}", delete(shopping::remove_staple))
}

/// The API, and the built web app in `web_dir` for every other path when given.
pub fn router(state: AppState, web_dir: Option<&Path>) -> Router {
    let api = api_routes();
    let mut app = Router::new()
        .nest(API, api.clone())
        .nest(OLD_API, api)
        .route("/api/auth/oidc/start", get(auth::oidc::start))
        .route("/api/auth/oidc/callback", get(auth::oidc::callback))
        .route("/api", any(|| async { ApiError::NotFound }))
        .route("/api/{*rest}", any(|| async { ApiError::NotFound }))
        // axum's own 405 has no body; this one says so in the API's error shape. The Allow header stays.
        .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ))
        .layer(middleware::from_fn(mark_deprecated))
        .with_state(state);
    if let Some(dir) = web_dir {
        app = app.fallback_service(web_app(dir));
    }
    app.layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
}

/// The built web app: its files, with any other path answered by index.html (status 200) so client-side
/// routes work on reload and the service worker can keep them for offline use.
fn web_app(dir: &Path) -> ServeDir<ServeFile> {
    ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")))
}
