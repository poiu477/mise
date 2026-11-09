//! What every API test needs: a throwaway database on the dev server named by DATABASE_URL, migrated and dropped
//! afterwards (even when the test fails), the router on top of it, and helpers to set up the owner, sign in and send
//! JSON with the session cookie.
//!
//! Without a database a test skips on a laptop but fails in CI, so a missing or unreachable database never passes
//! quietly there. Start a test with `let app = test_app!();`, or `test_app!(SomeApp::new())` for a wrapper that
//! returns `Option` the same way. `TestApp::with_demo()` starts one with the demo recipes loaded and a media folder
//! of its own (see `demo`).

// Each test file is its own crate and uses only some of these helpers.
#![allow(dead_code)]

pub mod demo;

use std::{path::PathBuf, sync::Arc};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode, header, request},
    response::Response,
};
use http_body_util::BodyExt;
use mise_api::auth::{Auth, AuthConfig, oidc::OidcConfig};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use tower::ServiceExt;

/// The owner made by first-run setup.
pub const OWNER: &str = "paul";
pub const OWNER_PASSWORD: &str = "a long password";

/// Every request carries this Host, which single sign-on builds its callback address from.
pub const HOST: &str = "mise.test";

/// The app under test, or return from the test when there is no database to run it on (outside CI only).
macro_rules! test_app {
    () => {
        test_app!($crate::common::TestApp::new())
    };
    ($start:expr) => {
        match $start.await {
            Some(app) => app,
            None => {
                eprintln!("DATABASE_URL is unset or unreachable; skipping");
                return;
            }
        }
    };
}

/// Who a request is from.
#[derive(Clone, Copy)]
pub enum As<'a> {
    Nobody,
    /// A browser with this session cookie.
    Cookie(&'a str),
    /// A script with this API token.
    Bearer(&'a str),
}

/// A response, read through.
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    /// The JSON body, or null when there is none.
    pub body: Value,
}

impl Reply {
    /// The session cookie this response sets: empty when it clears it.
    pub fn session(&self) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .find_map(|c| {
                c.strip_prefix("mise_session=")
                    .map(|v| v.split(';').next().unwrap_or_default().to_string())
            })
    }

    /// A response header as text, if it is set and readable.
    pub fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|h| h.to_str().ok())
    }
}

/// The router on a fresh, migrated database that is dropped with it.
pub struct TestApp {
    pub router: Router,
    /// The test database, for arranging data and checking what was stored.
    pub pool: PgPool,
    /// MEDIA_ROOT: where the server reads photos from.
    pub media_root: PathBuf,
    /// The folder at `media_root` when this test made one, removed with it.
    _media: Option<MediaDir>,
    // Last, so the pool is gone before the database is dropped.
    _db: Database,
}

impl TestApp {
    /// The app with local sign-in only.
    pub async fn new() -> Option<TestApp> {
        TestApp::start(None, None).await
    }

    /// The app with local sign-in and this single sign-on provider.
    pub async fn with_oidc(oidc: OidcConfig) -> Option<TestApp> {
        TestApp::start(Some(oidc), None).await
    }

    /// Make and migrate the database, then build the router with local sign-in, `oidc` if given, and photos read
    /// from `media` (or a folder that doesn't exist).
    async fn start(oidc: Option<OidcConfig>, media: Option<MediaDir>) -> Option<TestApp> {
        let db = Database::create().await?;
        let pool = mise_db::connect(&db.url()).await.unwrap();
        mise_db::migrate(&pool).await.unwrap();
        let config = AuthConfig {
            local: true,
            oidc,
            public_base_url: None,
            session_days: 30,
        };
        let media_root = media
            .as_ref()
            .map_or_else(|| PathBuf::from("./no-media"), |m| m.path.clone());
        let state = mise_api::AppState {
            pool: pool.clone(),
            media_root: media_root.clone(),
            auth: Arc::new(Auth::new(config)),
        };
        Some(TestApp {
            router: mise_api::router(state, None),
            pool,
            media_root,
            _media: media,
            _db: db,
        })
    }

    /// A request from this person, for adding headers or a body the other helpers don't cover.
    pub fn request(method: &str, path: &str, from: As<'_>) -> request::Builder {
        let b = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST);
        match from {
            As::Nobody => b,
            As::Cookie(c) => b.header(header::COOKIE, format!("mise_session={c}")),
            As::Bearer(t) => b.header(header::AUTHORIZATION, format!("Bearer {t}")),
        }
    }

    /// Run one request through the router and return the raw response.
    pub async fn oneshot(&self, req: Request<Body>) -> Response {
        self.router.clone().oneshot(req).await.unwrap()
    }

    /// Send a request, with a JSON body if there is one, and read the reply.
    pub async fn send(&self, method: &str, path: &str, from: As<'_>, body: Option<Value>) -> Reply {
        let b = TestApp::request(method, path, from);
        let req = match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string())),
            None => b.body(Body::empty()),
        };
        let res = self.oneshot(req.unwrap()).await;
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }

    /// Send as this browser and expect 200 OK; returns the JSON body.
    pub async fn json(&self, method: &str, path: &str, cookie: &str, body: Option<Value>) -> Value {
        let reply = self.send(method, path, As::Cookie(cookie), body).await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{method} {path}: {}",
            reply.body
        );
        reply.body
    }

    /// First-run setup: the owner's account, signed in. Returns its session cookie.
    pub async fn setup_owner(&self) -> String {
        let reply = self
            .send(
                "POST",
                "/api/auth/setup",
                As::Nobody,
                Some(json!({"username": OWNER, "password": OWNER_PASSWORD})),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "setup: {}", reply.body);
        reply.session().expect("setup signs in")
    }

    /// Sign in with a password and expect it to work. Returns the session cookie.
    pub async fn sign_in(&self, username: &str, password: &str) -> String {
        let reply = self
            .send(
                "POST",
                "/api/auth/login",
                As::Nobody,
                Some(json!({"username": username, "password": password})),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::OK,
            "sign in as {username}: {}",
            reply.body
        );
        reply.session().expect("signing in sets a session")
    }
}

/// A media folder made for one test under the system's temporary folder, and removed with it, even when the test
/// fails.
struct MediaDir {
    path: PathBuf,
}

impl MediaDir {
    fn create() -> MediaDir {
        let path =
            std::env::temp_dir().join(format!("mise-test-media-{}", uuid::Uuid::now_v7().simple()));
        std::fs::create_dir_all(&path).unwrap();
        MediaDir { path }
    }
}

impl Drop for MediaDir {
    /// Reported, not raised, as for the database: panicking while a failed test unwinds would abort the run.
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            eprintln!(
                "couldn't remove test media folder {}: {e}",
                self.path.display()
            );
        }
    }
}

/// How long dropping a test database may take before it is given up and reported.
const DROP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A database made for one test and dropped with it.
struct Database {
    /// DATABASE_URL, which the test connects to as an administrator.
    admin_url: String,
    name: String,
}

impl Database {
    /// None when there is no database to use, outside CI; in CI that is a failure.
    async fn create() -> Option<Database> {
        dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env")).ok();
        let in_ci = std::env::var_os("CI").is_some();
        let admin_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) if in_ci => panic!("DATABASE_URL must be set in CI"),
            Err(_) => return None,
        };
        let mut admin = match PgConnection::connect(&admin_url).await {
            Ok(conn) => conn,
            Err(e) if in_ci => panic!("can't connect to DATABASE_URL in CI: {e}"),
            Err(_) => return None,
        };
        let name = format!("mise_test_{}", uuid::Uuid::now_v7().simple());
        // The name is ours (a UUID), not input.
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&mut admin)
            .await
            .unwrap();
        admin.close().await.ok();
        Some(Database { admin_url, name })
    }

    /// The URL of this database: DATABASE_URL with the database name swapped.
    fn url(&self) -> String {
        let base = self.admin_url.rsplit_once('/').unwrap().0;
        format!("{base}/{}", self.name)
    }
}

impl Drop for Database {
    /// Drop runs outside async code, and also while a failed test unwinds, so the database is dropped on a thread
    /// with its own runtime. A failure is reported, not raised: panicking here would abort the test run. A server
    /// that stops answering gives up after `DROP_TIMEOUT` rather than hanging the run.
    fn drop(&mut self) {
        let (url, name) = (self.admin_url.clone(), self.name.clone());
        let dropped = std::thread::spawn(move || -> anyhow::Result<()> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            // The timeout is made inside the runtime: its timer needs one.
            rt.block_on(async {
                tokio::time::timeout(DROP_TIMEOUT, async {
                    let mut admin = PgConnection::connect(&url).await?;
                    // FORCE ends the connections the test's pool still holds.
                    sqlx::query(AssertSqlSafe(format!(
                        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
                    )))
                    .execute(&mut admin)
                    .await?;
                    admin.close().await?;
                    anyhow::Ok(())
                })
                .await
            })?
        })
        .join();
        match dropped {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("couldn't drop test database {}: {e:#}", self.name),
            Err(panic) => {
                let why = panic
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("the thread panicked");
                eprintln!("couldn't drop test database {}: {why}", self.name);
            }
        }
    }
}
