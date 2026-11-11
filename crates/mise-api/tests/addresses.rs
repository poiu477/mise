//! The API's addresses and its one error shape, through the router on the demo recipes: every documented route
//! answers under `/api/v1`, its old `/api` address answers the same and says it's deprecated, and every kind of
//! error carries both a message and a code.

#[macro_use]
mod common;

use axum::{
    body::Body,
    http::{StatusCode, header},
};
use common::{As, Reply, TestApp};
use serde_json::{Value, json};

/// A made-up id, the same on both addresses so their answers can be compared.
const SOME_ID: &str = "0190f4a2-0000-7000-8000-000000000000";

/// The demo install with its owner signed in, and a read-write token for the owner.
struct Demo {
    app: TestApp,
    cookie: String,
    token: String,
}

impl Demo {
    /// None when there is no database (outside CI).
    async fn new() -> Option<Demo> {
        let app = TestApp::with_demo().await?;
        let cookie = app.setup_owner().await;
        let made = app
            .json(
                "POST",
                "/api/v1/me/tokens",
                &cookie,
                Some(json!({"name": "addresses", "scope": "read_write"})),
            )
            .await;
        let token = made["token"].as_str().unwrap().to_string();
        Some(Demo { app, cookie, token })
    }
}

/// `path` from the API document with its parameters filled in: a real dish and version where the route finds one,
/// a made-up id elsewhere.
fn fill(path: &str, version: &str) -> String {
    let id = if path.contains("/variants/") {
        version
    } else {
        SOME_ID
    };
    path.replace("{slug}", "black-bean-tacos")
        .replace("{id}", id)
        .replace("{food_id}", SOME_ID)
        .replace("{size}", "400")
}

/// The `Deprecation` header, if the answer has one.
fn deprecation(reply: &Reply) -> Option<&str> {
    reply
        .headers
        .get("deprecation")
        .and_then(|h| h.to_str().ok())
}

#[tokio::test]
async fn every_old_address_answers_as_its_new_one_and_says_it_is_deprecated() {
    let demo = test_app!(Demo::new());
    let app = &demo.app;
    let doc = app
        .json("GET", "/api/v1/openapi.json", &demo.cookie, None)
        .await;
    let dish = app
        .json("GET", "/api/v1/dishes/black-bean-tacos", &demo.cookie, None)
        .await;
    let version = dish["default_variant"].as_str().unwrap().to_string();
    // Called once first, so the token's "last used" time is the same in both answers below.
    app.send("GET", "/api/v1/me", As::Bearer(&demo.token), None)
        .await;

    let mut routes: Vec<(String, String)> = vec![("get".into(), "/api/v1/openapi.json".into())];
    for (path, ops) in doc["paths"].as_object().expect("documented paths") {
        for method in ops.as_object().unwrap().keys() {
            routes.push((method.clone(), path.clone()));
        }
    }
    assert!(routes.len() > 30, "{} routes documented", routes.len());

    let mut compared = 0;
    for (method, path) in routes {
        let method = method.to_uppercase();
        let Some(rest) = path.strip_prefix("/api/v1") else {
            // Single sign-on's redirects keep their addresses, and nothing marks them deprecated.
            assert!(
                mise_api::PERMANENT.contains(&path.as_str()),
                "{path} is documented outside /api/v1"
            );
            let reply = app
                .send(&method, &fill(&path, &version), As::Nobody, None)
                .await;
            assert_eq!(deprecation(&reply), None, "{method} {path}");
            continue;
        };
        let new = fill(&path, &version);
        let old = format!("/api{}", fill(rest, &version));
        // Reads go as they are. Changes go with an empty body, refused alike at both addresses, or to a made-up id,
        // so neither call changes what the other sees. The token, unlike the cookie, survives signing out.
        let body = (method == "POST" || method == "PUT" || method == "PATCH").then(|| json!({}));
        let from = As::Bearer(&demo.token);
        let new_reply = app.send(&method, &new, from, body.clone()).await;
        let old_reply = app.send(&method, &old, from, body).await;

        assert_eq!(old_reply.status, new_reply.status, "{method} {old}");
        assert_eq!(old_reply.body, new_reply.body, "{method} {old}");
        assert_eq!(deprecation(&new_reply), None, "{method} {new}");
        if mise_api::PERMANENT.contains(&old.as_str()) {
            assert_eq!(deprecation(&old_reply), None, "{method} {old} stays");
            assert_eq!(old_reply.header(header::LINK), None, "{method} {old}");
        } else {
            assert_eq!(
                deprecation(&old_reply),
                Some(mise_api::DEPRECATED_SINCE),
                "{method} {old}"
            );
            assert_eq!(
                old_reply.header(header::LINK),
                Some(format!("<{new}>; rel=\"successor-version\"").as_str()),
                "{method} {old}"
            );
        }
        compared += 1;
    }
    assert!(compared > 30);
}

#[tokio::test]
async fn an_old_address_keeps_its_query_in_the_link_to_the_new_one() {
    let demo = test_app!(Demo::new());
    let reply = demo
        .app
        .send(
            "GET",
            "/api/dishes?q=tacos&limit=2",
            As::Cookie(&demo.cookie),
            None,
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.header(header::LINK),
        Some("</api/v1/dishes?q=tacos&limit=2>; rel=\"successor-version\"")
    );
}

#[tokio::test]
async fn the_health_check_answers_at_both_addresses_for_good() {
    let app = test_app!();
    for path in ["/api/health", "/api/v1/health"] {
        let reply = app.send("GET", path, As::Nobody, None).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(reply.body["status"], "ok", "{path}");
        assert_eq!(deprecation(&reply), None, "{path}");
    }
}

/// The reply must be an error of this status and code, with a message to show.
#[track_caller]
fn assert_error(reply: &Reply, status: StatusCode, code: &str, what: &str) {
    assert_eq!(reply.status, status, "{what}: {}", reply.body);
    assert_eq!(reply.body["code"], code, "{what}: {}", reply.body);
    assert!(
        reply.body["error"].as_str().is_some_and(|m| !m.is_empty()),
        "{what} has no message: {}",
        reply.body
    );
}

#[tokio::test]
async fn every_kind_of_error_has_a_message_and_a_code() {
    let demo = test_app!(Demo::new());
    let app = &demo.app;
    let me = As::Cookie(&demo.cookie);

    // Not found: a route that finds nothing, and an address no route answers.
    let r = app
        .send("GET", "/api/v1/dishes/no-such-dish", me, None)
        .await;
    assert_error(&r, StatusCode::NOT_FOUND, "not_found", "an unknown dish");
    for path in ["/api/v1/no-such-thing", "/api/no-such-thing", "/api"] {
        let r = app.send("GET", path, me, None).await;
        assert_error(&r, StatusCode::NOT_FOUND, "not_found", path);
    }

    // Refused input: a value the handler refuses, and each extractor's own refusals.
    let r = app
        .send(
            "POST",
            "/api/v1/shopping/extras",
            me,
            Some(json!({"name": "  "})),
        )
        .await;
    assert_error(&r, StatusCode::BAD_REQUEST, "bad_request", "a blank name");
    let r = app
        .send("GET", "/api/v1/variants/not-a-uuid", me, None)
        .await;
    assert_error(&r, StatusCode::BAD_REQUEST, "bad_request", "a bad path");
    let r = app.send("GET", "/api/v1/dishes?limit=lots", me, None).await;
    assert_error(&r, StatusCode::BAD_REQUEST, "bad_request", "a bad query");
    let r = app.send("GET", "/api/v1/dishes?q=a%00b", me, None).await;
    assert_error(&r, StatusCode::BAD_REQUEST, "bad_request", "a NUL");
    let bad_json = TestApp::request("POST", "/api/v1/shopping/extras", me)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"name\": "))
        .unwrap();
    let r = app.read(bad_json).await;
    assert_error(&r, StatusCode::BAD_REQUEST, "bad_request", "broken JSON");
    let a_form = TestApp::request("POST", "/api/v1/auth/login", As::Nobody)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("username=paul&password=x"))
        .unwrap();
    let r = app.read(a_form).await;
    assert_error(&r, StatusCode::BAD_REQUEST, "bad_request", "a form");
    let too_big = TestApp::request("POST", "/api/v1/shopping/extras", me)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            "{{\"name\": \"{}\"}}",
            "a".repeat(3 << 20)
        )))
        .unwrap();
    let r = app.read(too_big).await;
    assert_error(
        &r,
        StatusCode::PAYLOAD_TOO_LARGE,
        "payload_too_large",
        "3 MB",
    );

    // A method the address doesn't take: axum's own 405, in the API's shape, still saying what it does take.
    let r = app.send("DELETE", "/api/v1/dishes", me, None).await;
    assert_error(
        &r,
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "DELETE on search",
    );
    assert_eq!(r.header(header::ALLOW), Some("GET,HEAD"));

    // Signing in, and who may do what.
    let r = app.send("GET", "/api/v1/me", As::Nobody, None).await;
    assert_error(&r, StatusCode::UNAUTHORIZED, "unauthorized", "signed out");
    let r = app
        .send(
            "POST",
            "/api/v1/auth/login",
            As::Nobody,
            Some(json!({"username": common::OWNER, "password": "not the password"})),
        )
        .await;
    assert_error(
        &r,
        StatusCode::UNAUTHORIZED,
        "bad_credentials",
        "a wrong password",
    );
    let r = app
        .send(
            "POST",
            "/api/v1/me/tokens",
            As::Bearer(&demo.token),
            Some(json!({"name": "another", "scope": "read"})),
        )
        .await;
    assert_error(
        &r,
        StatusCode::FORBIDDEN,
        "forbidden",
        "a token minting a token",
    );
    let r = app
        .send(
            "POST",
            "/api/v1/auth/setup",
            As::Nobody,
            Some(json!({"username": "second", "password": "another long password"})),
        )
        .await;
    assert_error(&r, StatusCode::CONFLICT, "conflict", "a second setup");
    let elsewhere = TestApp::request("GET", "/api/v1/plan", me)
        .header("x-mise-account", SOME_ID)
        .body(Body::empty())
        .unwrap();
    let r = app.read(elsewhere).await;
    assert_error(
        &r,
        StatusCode::PRECONDITION_FAILED,
        "account_changed",
        "another account",
    );

    // Too many tries.
    let wrong = json!({"username": common::OWNER, "password": "still not it"});
    for _ in 0..5 {
        app.send(
            "POST",
            "/api/v1/auth/login",
            As::Nobody,
            Some(wrong.clone()),
        )
        .await;
    }
    let r = app
        .send("POST", "/api/v1/auth/login", As::Nobody, Some(wrong))
        .await;
    assert_error(
        &r,
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_requests",
        "a locked account",
    );

    // The old addresses answer in the same shape.
    let r = app.send("GET", "/api/variants/not-a-uuid", me, None).await;
    assert_error(
        &r,
        StatusCode::BAD_REQUEST,
        "bad_request",
        "an old bad path",
    );
}

#[tokio::test]
async fn an_error_body_is_documented_with_its_codes() {
    let demo = test_app!(Demo::new());
    let doc = demo
        .app
        .json("GET", "/api/v1/openapi.json", &demo.cookie, None)
        .await;
    let schemas = &doc["components"]["schemas"];
    let body: Vec<&str> = schemas["ErrorBody"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(body, ["error", "code"]);
    let codes: Vec<&str> = schemas["ErrorCode"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for code in [
        "not_found",
        "bad_request",
        "unauthorized",
        "bad_credentials",
        "forbidden",
        "conflict",
        "account_changed",
        "too_many_requests",
        "internal",
    ] {
        assert!(codes.contains(&code), "{code} in {codes:?}");
    }
}
