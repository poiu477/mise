//! Sign-in end to end through the router, each test on its own throwaway database (see `common`).

#[macro_use]
mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{As, OWNER, OWNER_PASSWORD, TestApp};
use serde_json::json;

#[tokio::test]
async fn fresh_install_is_locked_until_setup() {
    let app = test_app!();
    assert_eq!(
        app.send("GET", "/api/health", As::Nobody, None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send("GET", "/api/dishes", As::Nobody, None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(
            "GET",
            "/api/media/00000000-0000-0000-0000-000000000000/400",
            As::Nobody,
            None
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    let cfg = app
        .send("GET", "/api/auth/config", As::Nobody, None)
        .await
        .body;
    assert_eq!(cfg["setup_required"], true);
    assert_eq!(cfg["local"], true);

    let cookie = app.setup_owner().await;
    let me = app.json("GET", "/api/me", &cookie, None).await;
    assert_eq!(me["is_admin"], true);
    assert_eq!(me["household"]["name"], "Home");
    app.json("GET", "/api/dishes", &cookie, None).await;

    // Setup only once.
    let reply = app
        .send(
            "POST",
            "/api/auth/setup",
            As::Nobody,
            Some(json!({"username": "eve", "password": "another password"})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(
        app.send("GET", "/api/auth/config", As::Nobody, None)
            .await
            .body["setup_required"],
        false
    );
}

#[tokio::test]
async fn login_logout_and_lockout() {
    let app = test_app!();
    app.setup_owner().await;
    let creds = |p: &str| Some(json!({"username": "PAUL", "password": p}));
    let reply = app
        .send("POST", "/api/auth/login", As::Nobody, creds(OWNER_PASSWORD))
        .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "usernames are case-insensitive"
    );
    let cookie = reply.session().unwrap();
    app.json("GET", "/api/me", &cookie, None).await;

    let reply = app
        .send("POST", "/api/auth/logout", As::Cookie(&cookie), None)
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert_eq!(
        reply.session().as_deref(),
        Some(""),
        "the cookie is cleared"
    );
    assert_eq!(
        app.send("GET", "/api/me", As::Cookie(&cookie), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the session is gone"
    );

    for _ in 0..5 {
        assert_eq!(
            app.send(
                "POST",
                "/api/auth/login",
                As::Nobody,
                creds("wrong password")
            )
            .await
            .status,
            StatusCode::UNAUTHORIZED
        );
    }
    let reply = app
        .send("POST", "/api/auth/login", As::Nobody, creds(OWNER_PASSWORD))
        .await;
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "locked even with the right password"
    );
    assert!(
        reply
            .header(header::RETRY_AFTER)
            .is_some_and(|r| r.parse::<u64>().unwrap() > 0)
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/auth/login",
            As::Nobody,
            Some(json!({"username": "nobody", "password": "whatever123"}))
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn forms_from_other_sites_are_refused() {
    let app = test_app!();
    app.setup_owner().await;
    let req = TestApp::request("POST", "/api/auth/login", As::Nobody)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("username=paul&password=a+long+password"))
        .unwrap();
    assert_eq!(
        app.oneshot(req).await.status(),
        StatusCode::BAD_REQUEST,
        "only JSON bodies are accepted"
    );
}

#[tokio::test]
async fn api_tokens_and_their_scopes() {
    let app = test_app!();
    let cookie = app.setup_owner().await;
    let read = app
        .json(
            "POST",
            "/api/me/tokens",
            &cookie,
            Some(json!({"name": "grocy", "scope": "read"})),
        )
        .await;
    let read = read["token"].as_str().unwrap().to_string();
    assert!(read.starts_with("mise_"));
    let rw = app
        .send(
            "POST",
            "/api/me/tokens",
            As::Cookie(&cookie),
            Some(json!({"name": "script", "scope": "read_write"})),
        )
        .await
        .body;
    let rw = rw["token"].as_str().unwrap().to_string();

    assert_eq!(
        app.send("GET", "/api/me", As::Bearer(&read), None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/households",
            As::Bearer(&read),
            Some(json!({"name": "x"}))
        )
        .await
        .status,
        StatusCode::FORBIDDEN,
        "read tokens only read"
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/households",
            As::Bearer(&rw),
            Some(json!({"name": "Neighbours"}))
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/me/tokens",
            As::Bearer(&rw),
            Some(json!({"name": "y", "scope": "read"}))
        )
        .await
        .status,
        StatusCode::FORBIDDEN,
        "tokens can't mint tokens"
    );
    assert_eq!(
        app.send("GET", "/api/me", As::Bearer("mise_not-a-real-token"), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send("GET", "/api/me", As::Bearer("garbage"), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );

    let list = app.json("GET", "/api/me/tokens", &cookie, None).await;
    let id = list
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "grocy")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !list.to_string().contains("mise_"),
        "listing never shows a token"
    );
    assert_eq!(
        app.send(
            "DELETE",
            &format!("/api/me/tokens/{id}"),
            As::Cookie(&cookie),
            None
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send("GET", "/api/me", As::Bearer(&read), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "revoked"
    );
}

#[tokio::test]
async fn admins_manage_accounts() {
    let app = test_app!();
    let admin = app.setup_owner().await;
    let me = app.json("GET", "/api/me", &admin, None).await;
    let home = me["household"]["id"].as_str().unwrap().to_string();
    let admin_id = me["id"].as_str().unwrap().to_string();

    let kid = app
        .json(
            "POST",
            "/api/users",
            &admin,
            Some(json!({"username": "sam", "password": "sams password", "household_id": home})),
        )
        .await;
    assert_eq!(kid["household"]["id"], home.as_str());
    let kid_id = kid["id"].as_str().unwrap().to_string();
    assert_eq!(
        app.send(
            "POST",
            "/api/users",
            As::Cookie(&admin),
            Some(json!({"username": "SAM", "password": "whatever12"}))
        )
        .await
        .status,
        StatusCode::CONFLICT
    );

    let sam = app.sign_in("sam", "sams password").await;
    assert_eq!(
        app.send("GET", "/api/users", As::Cookie(&sam), None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.json("GET", "/api/users", &admin, None)
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // The last admin stays an admin, and nobody turns off their own account.
    let reply = app
        .send(
            "PATCH",
            &format!("/api/users/{admin_id}"),
            As::Cookie(&admin),
            Some(json!({"is_admin": false})),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert_eq!(
        app.send(
            "PATCH",
            &format!("/api/users/{admin_id}"),
            As::Cookie(&admin),
            Some(json!({"disabled": true}))
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );

    // Turning an account off signs it out.
    app.json(
        "PATCH",
        &format!("/api/users/{kid_id}"),
        &admin,
        Some(json!({"disabled": true})),
    )
    .await;
    assert_eq!(
        app.send("GET", "/api/me", As::Cookie(&sam), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/auth/login",
            As::Nobody,
            Some(json!({"username": "sam", "password": "sams password"}))
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn changing_the_password_signs_out_other_browsers() {
    let app = test_app!();
    let first = app.setup_owner().await;
    let second = app.sign_in(OWNER, OWNER_PASSWORD).await;
    assert_eq!(
        app.send(
            "POST",
            "/api/me/password",
            As::Cookie(&first),
            Some(json!({"current": "wrong", "new": "brand new password"}))
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.send(
            "POST",
            "/api/me/password",
            As::Cookie(&first),
            Some(json!({"current": OWNER_PASSWORD, "new": "brand new password"}))
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.send("GET", "/api/me", As::Cookie(&first), None)
            .await
            .status,
        StatusCode::OK,
        "this browser stays in"
    );
    assert_eq!(
        app.send("GET", "/api/me", As::Cookie(&second), None)
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "others are out"
    );
    app.sign_in(OWNER, "brand new password").await;
}

#[tokio::test]
async fn work_meant_for_another_account_is_refused() {
    let app = test_app!();
    let cookie = app.setup_owner().await;
    let me = app.json("GET", "/api/me", &cookie, None).await;
    let me = me["id"].as_str().unwrap().to_string();
    let as_account = |account: &str| {
        TestApp::request("GET", "/api/me", As::Cookie(&cookie))
            .header("x-mise-account", account)
            .body(Body::empty())
            .unwrap()
    };
    let status = |req: Request<Body>| async { app.oneshot(req).await.status() };
    // The page's account is the one signed in: fine.
    assert_eq!(status(as_account(&me)).await, StatusCode::OK);
    // Someone else signed in since (in another tab, with the shared cookie), or the header is nonsense: refused.
    assert_eq!(
        status(as_account(&uuid::Uuid::now_v7().to_string())).await,
        StatusCode::PRECONDITION_FAILED
    );
    assert_eq!(
        status(as_account("not-an-id")).await,
        StatusCode::PRECONDITION_FAILED
    );
    // Without the header (scripts, integrations) nothing changes.
    app.json("GET", "/api/me", &cookie, None).await;
}
