//! Seeing what the server does, through the router: the readiness check answers while the database does and 503
//! once it doesn't; every answer carries a request id, the proxy's when it sent a plain one; and requests are
//! counted by the route they matched, not by their address.

#[macro_use]
mod common;

use axum::body::Body;
use axum::http::StatusCode;
use common::{As, TestApp};

#[tokio::test]
async fn ready_answers_while_the_database_does() {
    let app = test_app!(TestApp::new());
    // No sign-in needed: a container orchestrator or proxy asks it.
    let reply = app.send("GET", "/api/v1/ready", As::Nobody, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.body["status"], "ready");

    app.pool.close().await;
    let reply = app.send("GET", "/api/v1/ready", As::Nobody, None).await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.body["code"], "unavailable");
    assert_eq!(reply.body["error"], "the database isn't answering");
    // The process is still up.
    let health = app.send("GET", "/api/v1/health", As::Nobody, None).await;
    assert_eq!(health.status, StatusCode::OK);
}

#[tokio::test]
async fn every_answer_carries_a_request_id() {
    let app = test_app!(TestApp::new());
    let id = |reply: &common::Reply| {
        reply
            .header("x-request-id".parse().unwrap())
            .map(str::to_owned)
    };

    let ok = app.send("GET", "/api/v1/health", As::Nobody, None).await;
    let refused = app.send("GET", "/api/v1/me", As::Nobody, None).await;
    let unknown = app
        .send("GET", "/api/v1/nothing-here", As::Nobody, None)
        .await;
    let mut seen = Vec::new();
    for reply in [&ok, &refused, &unknown] {
        let id = id(reply).expect("a request id");
        uuid::Uuid::parse_str(&id).unwrap_or_else(|e| panic!("{id}: {e}"));
        seen.push(id);
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 3, "each request has its own");

    // A proxy's own id is kept, so its log and mise's name the request alike; one that could break a log line isn't.
    let with = |sent: &str| {
        TestApp::request("GET", "/api/v1/health", As::Nobody)
            .header("x-request-id", sent)
            .body(Body::empty())
            .unwrap()
    };
    let kept = app.read(with("edge-7f3a.42")).await;
    assert_eq!(id(&kept).as_deref(), Some("edge-7f3a.42"));
    let replaced = app.read(with("two words \" and a quote")).await;
    let replaced = id(&replaced).unwrap();
    assert!(uuid::Uuid::parse_str(&replaced).is_ok(), "{replaced}");
}

#[tokio::test]
async fn requests_are_counted_by_the_route_they_matched() {
    let app = test_app!(TestApp::with_demo());
    let cookie = app.setup_owner().await;
    for slug in ["weeknight-tomato-pasta", "black-bean-tacos", "no-such-dish"] {
        app.send(
            "GET",
            &format!("/api/v1/dishes/{slug}"),
            As::Cookie(&cookie),
            None,
        )
        .await;
    }
    app.send("GET", "/api/v1/nothing-here", As::Cookie(&cookie), None)
        .await;
    let text = app.metrics.render();
    let count = |labels: &str| {
        let line = format!("mise_http_requests_total{{{labels}}} ");
        text.lines()
            .find_map(|l| l.strip_prefix(&line))
            .unwrap_or_else(|| panic!("no {line} in\n{text}"))
            .to_owned()
    };
    // One series for every dish, not one each.
    assert_eq!(
        count(r#"method="GET",route="/api/v1/dishes/{slug}",status="200""#),
        "2"
    );
    assert_eq!(
        count(r#"method="GET",route="/api/v1/dishes/{slug}",status="404""#),
        "1"
    );
    assert_eq!(
        count(r#"method="GET",route="/api/{*rest}",status="404""#),
        "1"
    );
    assert!(text.contains(
        "mise_http_request_duration_seconds_count{method=\"GET\",route=\"/api/v1/dishes/{slug}\",status=\"200\"} 2"
    ));
    // A dish's address, or anything else a person typed, never becomes a label.
    assert!(!text.contains("weeknight-tomato-pasta") && !text.contains("nothing-here"));
}
