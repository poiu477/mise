//! What keeps the app safe in a browser, through the router: the security headers on a page of the web app and on
//! an API response, and requests another site makes a signed-in browser send (sign-out, a DELETE, a fork) refused
//! and harmless, as docs/architecture.md's "Requests from other sites" says.

#[macro_use]
mod common;

use axum::body::Body;
use axum::http::{StatusCode, header};
use common::{As, TestApp};
use serde_json::json;

/// The headers every response carries, and their values.
const HEADERS: &[(&str, &str)] = &[
    (
        "content-security-policy",
        mise_api::security::CONTENT_SECURITY_POLICY,
    ),
    ("x-content-type-options", "nosniff"),
    ("referrer-policy", "same-origin"),
    ("x-frame-options", "DENY"),
];

#[tokio::test]
async fn a_page_and_an_api_response_carry_the_security_headers() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><title>mise</title><div id=root></div>",
    )
    .unwrap();
    let app = test_app!(TestApp::with_web_app(web.path()));

    let page = app
        .oneshot(
            TestApp::request("GET", "/dishes/anything", As::Nobody)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(page.status(), StatusCode::OK);
    let api = app
        .send("GET", "/api/v1/auth/config", As::Nobody, None)
        .await;
    assert_eq!(api.status, StatusCode::OK);
    let refused = app.send("GET", "/api/v1/me", As::Nobody, None).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);

    for (name, value) in HEADERS {
        assert_eq!(
            page.headers().get(*name).and_then(|v| v.to_str().ok()),
            Some(*value),
            "the page's {name}"
        );
        assert_eq!(
            api.header(name.parse().unwrap()),
            Some(*value),
            "the API's {name}"
        );
        assert_eq!(
            refused.header(name.parse().unwrap()),
            Some(*value),
            "an error's {name}"
        );
    }
    // The policy lets the app load only from itself, and lets no other site frame it.
    let policy = mise_api::security::CONTENT_SECURITY_POLICY;
    assert!(policy.contains("default-src 'self'") && policy.contains("frame-ancestors 'none'"));
    assert!(!policy.contains("unsafe-inline") && !policy.contains("unsafe-eval"));
}

/// A request the way another site's page makes a signed-in browser send it: the session cookie along (a sibling on
/// the same domain gets it past `SameSite=Lax`), no JSON body, no header of mise's own, labelled by the browser.
fn from_another_site(
    method: &str,
    path: &str,
    cookie: &str,
    site: &str,
) -> axum::http::Request<Body> {
    TestApp::request(method, path, As::Cookie(cookie))
        .header(header::CONTENT_TYPE, "text/plain")
        .header("sec-fetch-site", site)
        .header(header::ORIGIN, "https://other.example")
        .body(Body::from("x"))
        .unwrap()
}

#[tokio::test]
async fn sign_out_from_another_site_is_refused_and_signs_nobody_out() {
    let app = test_app!(TestApp::new());
    let cookie = app.setup_owner().await;
    for site in ["cross-site", "same-site"] {
        let reply = app
            .read(from_another_site(
                "POST",
                "/api/v1/auth/logout",
                &cookie,
                site,
            ))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{site}");
        assert_eq!(reply.body["code"], "forbidden");
        assert!(reply.session().is_none(), "the cookie isn't cleared");
    }
    // An old browser that sends no label: its Origin isn't this server.
    let req = TestApp::request("POST", "/api/v1/auth/logout", As::Cookie(&cookie))
        .header(header::ORIGIN, "https://other.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.read(req).await.status, StatusCode::FORBIDDEN);
    // Still signed in.
    app.json("GET", "/api/v1/me", &cookie, None).await;

    // From mise's own page it works.
    let req = TestApp::request("POST", "/api/v1/auth/logout", As::Cookie(&cookie))
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.read(req).await.status, StatusCode::NO_CONTENT);
    let after = app
        .send("GET", "/api/v1/me", As::Cookie(&cookie), None)
        .await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_delete_from_another_site_is_refused_and_removes_nothing() {
    let app = test_app!(TestApp::new());
    let cookie = app.setup_owner().await;
    let made = app
        .json(
            "POST",
            "/api/v1/me/tokens",
            &cookie,
            Some(json!({"name": "script", "scope": "read"})),
        )
        .await;
    let path = format!("/api/v1/me/tokens/{}", made["info"]["id"].as_str().unwrap());
    let reply = app
        .read(from_another_site("DELETE", &path, &cookie, "cross-site"))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let tokens = app.json("GET", "/api/v1/me/tokens", &cookie, None).await;
    assert_eq!(
        tokens.as_array().unwrap().len(),
        1,
        "the token is still there"
    );

    // The `mise` command or a script, which sends neither label nor Origin, is no browser page: left alone.
    let reply = app.send("DELETE", &path, As::Cookie(&cookie), None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_fork_from_another_site_is_refused_and_makes_nothing() {
    let app = test_app!(TestApp::with_demo());
    let cookie = app.setup_owner().await;
    let dish = app
        .json(
            "GET",
            "/api/v1/dishes/weeknight-tomato-pasta",
            &cookie,
            None,
        )
        .await;
    let version = dish["variants"][0]["id"].as_str().unwrap();
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM variants WHERE origin = 'user'")
            .fetch_one(&app.pool)
            .await
            .unwrap()
    };
    let before = count().await;
    let reply = app
        .read(from_another_site(
            "POST",
            &format!("/api/v1/variants/{version}/fork"),
            &cookie,
            "cross-site",
        ))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(count().await, before);
}

#[tokio::test]
async fn reading_from_another_site_is_left_to_the_cookie_and_cors() {
    // A link from another site to a dish is a person following it: reading is never refused here.
    let app = test_app!(TestApp::with_demo());
    let cookie = app.setup_owner().await;
    let req = TestApp::request("GET", "/api/v1/dishes", As::Cookie(&cookie))
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())
        .unwrap();
    let reply = app.read(req).await;
    assert_eq!(reply.status, StatusCode::OK);
    // And no CORS header lets another site's script read the answer.
    assert!(reply.header(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}
