//! Sign-in end to end through the router, each test on a fresh database created on the dev server from
//! DATABASE_URL (skipped when it is not set).

use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use mise_api::auth::{Auth, AuthConfig};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    pool: PgPool,
    admin: PgPool,
    db: String,
}

impl TestApp {
    async fn new() -> Option<TestApp> {
        dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env")).ok();
        let url = std::env::var("DATABASE_URL").ok()?;
        let admin = mise_db::connect(&url).await.ok()?;
        let db = format!("mise_test_{}", uuid::Uuid::now_v7().simple());
        // The name is ours (a UUID), not input.
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {db}"))).execute(&admin).await.unwrap();
        let base = url.rsplit_once('/').unwrap().0;
        let pool = mise_db::connect(&format!("{base}/{db}")).await.unwrap();
        mise_db::migrate(&pool).await.unwrap();
        let config = AuthConfig { local: true, oidc: None, public_base_url: None, session_days: 30 };
        let state = mise_api::AppState { pool: pool.clone(), media_root: "./no-media".into(), auth: Arc::new(Auth::new(config)) };
        Some(TestApp { router: mise_api::router(state, None), pool, admin, db })
    }

    async fn call(&self, method: &str, path: &str, auth: Auth_<'_>, body: Option<Value>) -> (StatusCode, Value, Option<String>, Option<String>) {
        let mut b = Request::builder().method(method).uri(path);
        match auth {
            Auth_::None => {}
            Auth_::Cookie(c) => b = b.header(header::COOKIE, format!("mise_session={c}")),
            Auth_::Bearer(t) => b = b.header(header::AUTHORIZATION, format!("Bearer {t}")),
        }
        let req = match body {
            Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .find_map(|c| c.strip_prefix("mise_session=").map(|v| v.split(';').next().unwrap().to_string()));
        let retry = res.headers().get(header::RETRY_AFTER).and_then(|h| h.to_str().ok()).map(str::to_string);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null), cookie, retry)
    }

    async fn setup(&self) -> String {
        let (s, _, cookie, _) = self.call("POST", "/api/auth/setup", Auth_::None, Some(json!({"username": "paul", "password": "a long password"}))).await;
        assert_eq!(s, StatusCode::OK);
        cookie.expect("setup signs in")
    }

    async fn drop(self) {
        self.pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.db))).execute(&self.admin).await;
    }
}

#[derive(Clone, Copy)]
enum Auth_<'a> {
    None,
    Cookie(&'a str),
    Bearer(&'a str),
}

macro_rules! app {
    () => {
        match TestApp::new().await {
            Some(a) => a,
            None => {
                eprintln!("DATABASE_URL not set; skipping");
                return;
            }
        }
    };
}

#[tokio::test]
async fn fresh_install_is_locked_until_setup() {
    let app = app!();
    assert_eq!(app.call("GET", "/api/health", Auth_::None, None).await.0, StatusCode::OK);
    assert_eq!(app.call("GET", "/api/dishes", Auth_::None, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(app.call("GET", "/api/media/00000000-0000-0000-0000-000000000000/400", Auth_::None, None).await.0, StatusCode::UNAUTHORIZED);
    let (_, cfg, _, _) = app.call("GET", "/api/auth/config", Auth_::None, None).await;
    assert_eq!(cfg["setup_required"], true);
    assert_eq!(cfg["local"], true);

    let cookie = app.setup().await;
    let (s, me, _, _) = app.call("GET", "/api/me", Auth_::Cookie(&cookie), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(me["is_admin"], true);
    assert_eq!(me["household"]["name"], "Home");
    assert_eq!(app.call("GET", "/api/dishes", Auth_::Cookie(&cookie), None).await.0, StatusCode::OK);

    // Setup only once.
    let (s, _, _, _) = app.call("POST", "/api/auth/setup", Auth_::None, Some(json!({"username": "eve", "password": "another password"}))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(app.call("GET", "/api/auth/config", Auth_::None, None).await.1["setup_required"], false);
    app.drop().await;
}

#[tokio::test]
async fn login_logout_and_lockout() {
    let app = app!();
    app.setup().await;
    let creds = |p: &str| Some(json!({"username": "PAUL", "password": p}));
    let (s, _, cookie, _) = app.call("POST", "/api/auth/login", Auth_::None, creds("a long password")).await;
    assert_eq!(s, StatusCode::OK, "usernames are case-insensitive");
    let cookie = cookie.unwrap();
    assert_eq!(app.call("GET", "/api/me", Auth_::Cookie(&cookie), None).await.0, StatusCode::OK);

    let (s, _, cleared, _) = app.call("POST", "/api/auth/logout", Auth_::Cookie(&cookie), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(cleared.as_deref(), Some(""), "the cookie is cleared");
    assert_eq!(app.call("GET", "/api/me", Auth_::Cookie(&cookie), None).await.0, StatusCode::UNAUTHORIZED, "the session is gone");

    for _ in 0..5 {
        assert_eq!(app.call("POST", "/api/auth/login", Auth_::None, creds("wrong password")).await.0, StatusCode::UNAUTHORIZED);
    }
    let (s, _, _, retry) = app.call("POST", "/api/auth/login", Auth_::None, creds("a long password")).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "locked even with the right password");
    assert!(retry.is_some_and(|r| r.parse::<u64>().unwrap() > 0));
    assert_eq!(app.call("POST", "/api/auth/login", Auth_::None, Some(json!({"username": "nobody", "password": "whatever123"}))).await.0, StatusCode::UNAUTHORIZED);
    app.drop().await;
}

#[tokio::test]
async fn forms_from_other_sites_are_refused() {
    let app = app!();
    app.setup().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("username=paul&password=a+long+password"))
        .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "only JSON bodies are accepted");
    app.drop().await;
}

#[tokio::test]
async fn api_tokens_and_their_scopes() {
    let app = app!();
    let cookie = app.setup().await;
    let (s, read, _, _) = app.call("POST", "/api/me/tokens", Auth_::Cookie(&cookie), Some(json!({"name": "grocy", "scope": "read"}))).await;
    assert_eq!(s, StatusCode::OK);
    let read = read["token"].as_str().unwrap().to_string();
    assert!(read.starts_with("mise_"));
    let (_, rw, _, _) = app.call("POST", "/api/me/tokens", Auth_::Cookie(&cookie), Some(json!({"name": "script", "scope": "read_write"}))).await;
    let rw = rw["token"].as_str().unwrap().to_string();

    assert_eq!(app.call("GET", "/api/me", Auth_::Bearer(&read), None).await.0, StatusCode::OK);
    assert_eq!(app.call("POST", "/api/households", Auth_::Bearer(&read), Some(json!({"name": "x"}))).await.0, StatusCode::FORBIDDEN, "read tokens only read");
    assert_eq!(app.call("POST", "/api/households", Auth_::Bearer(&rw), Some(json!({"name": "Neighbours"}))).await.0, StatusCode::OK);
    assert_eq!(app.call("POST", "/api/me/tokens", Auth_::Bearer(&rw), Some(json!({"name": "y", "scope": "read"}))).await.0, StatusCode::FORBIDDEN, "tokens can't mint tokens");
    assert_eq!(app.call("GET", "/api/me", Auth_::Bearer("mise_not-a-real-token"), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(app.call("GET", "/api/me", Auth_::Bearer("garbage"), None).await.0, StatusCode::UNAUTHORIZED);

    let (_, list, _, _) = app.call("GET", "/api/me/tokens", Auth_::Cookie(&cookie), None).await;
    let id = list.as_array().unwrap().iter().find(|t| t["name"] == "grocy").unwrap()["id"].as_str().unwrap().to_string();
    assert!(list.to_string().find("mise_").is_none(), "listing never shows a token");
    assert_eq!(app.call("DELETE", &format!("/api/me/tokens/{id}"), Auth_::Cookie(&cookie), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app.call("GET", "/api/me", Auth_::Bearer(&read), None).await.0, StatusCode::UNAUTHORIZED, "revoked");
    app.drop().await;
}

#[tokio::test]
async fn admins_manage_accounts() {
    let app = app!();
    let admin = app.setup().await;
    let (s, me, _, _) = app.call("GET", "/api/me", Auth_::Cookie(&admin), None).await;
    assert_eq!(s, StatusCode::OK);
    let home = me["household"]["id"].as_str().unwrap().to_string();
    let admin_id = me["id"].as_str().unwrap().to_string();

    let (s, kid, _, _) = app
        .call("POST", "/api/users", Auth_::Cookie(&admin), Some(json!({"username": "sam", "password": "sams password", "household_id": home})))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(kid["household"]["id"], home.as_str());
    let kid_id = kid["id"].as_str().unwrap().to_string();
    assert_eq!(app.call("POST", "/api/users", Auth_::Cookie(&admin), Some(json!({"username": "SAM", "password": "whatever12"}))).await.0, StatusCode::CONFLICT);

    let (_, _, sam, _) = app.call("POST", "/api/auth/login", Auth_::None, Some(json!({"username": "sam", "password": "sams password"}))).await;
    let sam = sam.unwrap();
    assert_eq!(app.call("GET", "/api/users", Auth_::Cookie(&sam), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app.call("GET", "/api/users", Auth_::Cookie(&admin), None).await.1.as_array().unwrap().len(), 2);

    // The last admin stays an admin, and nobody turns off their own account.
    let (s, body, _, _) = app.call("PATCH", &format!("/api/users/{admin_id}"), Auth_::Cookie(&admin), Some(json!({"is_admin": false}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(app.call("PATCH", &format!("/api/users/{admin_id}"), Auth_::Cookie(&admin), Some(json!({"disabled": true}))).await.0, StatusCode::BAD_REQUEST);

    // Turning an account off signs it out.
    assert_eq!(app.call("PATCH", &format!("/api/users/{kid_id}"), Auth_::Cookie(&admin), Some(json!({"disabled": true}))).await.0, StatusCode::OK);
    assert_eq!(app.call("GET", "/api/me", Auth_::Cookie(&sam), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(app.call("POST", "/api/auth/login", Auth_::None, Some(json!({"username": "sam", "password": "sams password"}))).await.0, StatusCode::FORBIDDEN);
    app.drop().await;
}

#[tokio::test]
async fn changing_the_password_signs_out_other_browsers() {
    let app = app!();
    let first = app.setup().await;
    let (_, _, second, _) = app.call("POST", "/api/auth/login", Auth_::None, Some(json!({"username": "paul", "password": "a long password"}))).await;
    let second = second.unwrap();
    assert_eq!(
        app.call("POST", "/api/me/password", Auth_::Cookie(&first), Some(json!({"current": "wrong", "new": "brand new password"}))).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.call("POST", "/api/me/password", Auth_::Cookie(&first), Some(json!({"current": "a long password", "new": "brand new password"}))).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.call("GET", "/api/me", Auth_::Cookie(&first), None).await.0, StatusCode::OK, "this browser stays in");
    assert_eq!(app.call("GET", "/api/me", Auth_::Cookie(&second), None).await.0, StatusCode::UNAUTHORIZED, "others are out");
    assert_eq!(app.call("POST", "/api/auth/login", Auth_::None, Some(json!({"username": "paul", "password": "brand new password"}))).await.0, StatusCode::OK);
    app.drop().await;
}
