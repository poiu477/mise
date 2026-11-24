//! Seeing what the server does: a request id on every request, in its log lines and its answer; a readiness check
//! that the database answers; and request counts and durations by route, in Prometheus's text format, for a
//! metrics port of their own (`MISE_METRICS_BIND`) that the API's port never serves.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderName, HeaderValue, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use tracing::Span;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
};

// ---------------------------------------------------------------------------------------------------------
// Request ids

/// The header a request's id travels in, both ways.
pub const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// A request's id, in its extensions for the log span to read.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Whether an id a proxy sent is one to keep: short, and only letters, digits and `-_.:`, so it can't break a log
/// line or a header.
fn acceptable(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

/// Gives every request an id: the one a reverse proxy sent in `X-Request-Id` when it is acceptable, so the proxy's
/// log and mise's agree, or a new UUID. The answer carries it back in the same header.
pub async fn request_id(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(&REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .filter(|id| acceptable(id))
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let value = HeaderValue::from_str(&id).expect("an acceptable id or a UUID is a header value");
    req.headers_mut().insert(REQUEST_ID, value.clone());
    req.extensions_mut().insert(RequestId(id));
    let mut res = next.run(req).await;
    res.headers_mut().insert(REQUEST_ID, value);
    res
}

/// The span a request's log lines are written in: its id, method and path. The query is left out: it can hold a
/// kitchen link's one-time code.
pub fn span<B>(req: &axum::http::Request<B>) -> Span {
    let id = req
        .extensions()
        .get::<RequestId>()
        .map_or("", |r| r.0.as_str());
    tracing::info_span!(
        "request",
        request_id = %id,
        method = %req.method(),
        path = %req.uri().path(),
    )
}

// ---------------------------------------------------------------------------------------------------------
// Readiness

/// The server can serve requests.
#[derive(Serialize, ToSchema)]
pub struct Ready {
    /// Always `ready`.
    pub status: &'static str,
}

/// How long readiness waits for the database.
const READY_WITHIN: Duration = Duration::from_secs(2);

/// Readiness: whether the server can serve requests, which needs its database to answer within two seconds. A
/// container orchestrator or a proxy sends traffic only while this answers 200; `/api/v1/health` says only that the
/// process is up.
#[utoipa::path(get, path = "/api/v1/ready", operation_id = "ready", security(()),
    responses(
        (status = 200, description = "The server and its database are answering", body = Ready),
        (status = 503, description = "The database isn't answering", body = crate::error::ErrorBody)
    )
)]
pub async fn ready(State(s): State<AppState>) -> ApiResult<Json<Ready>> {
    match tokio::time::timeout(READY_WITHIN, sqlx::query("SELECT 1").execute(&s.pool)).await {
        Ok(Ok(_)) => Ok(Json(Ready { status: "ready" })),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "not ready: the database isn't answering");
            Err(ApiError::Unavailable("the database isn't answering".into()))
        }
        Err(_) => {
            tracing::warn!("not ready: the database took over two seconds to answer");
            Err(ApiError::Unavailable(
                "the database didn't answer within two seconds".into(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------------------------------------
// Metrics

/// Upper bounds of the duration histogram's buckets, in seconds.
const BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// One series: requests with one method, route and status.
#[derive(Clone, Default)]
struct Series {
    count: u64,
    seconds: f64,
    /// How many took no longer than each bucket's bound (not cumulative; `render` adds them up).
    buckets: [u64; BUCKETS.len()],
}

/// Request counts and durations since the server started, by method, route and status.
#[derive(Default)]
pub struct Metrics {
    series: Mutex<BTreeMap<(String, String, u16), Series>>,
}

impl Metrics {
    /// Counts a request that took `seconds`.
    pub fn record(&self, method: &str, route: &str, status: u16, seconds: f64) {
        let mut series = self.series.lock().expect("metrics lock");
        let s = series
            .entry((method.to_owned(), route.to_owned(), status))
            .or_default();
        s.count += 1;
        s.seconds += seconds;
        if let Some(i) = BUCKETS.iter().position(|b| seconds <= *b) {
            s.buckets[i] += 1;
        }
    }

    /// Everything counted, in Prometheus's text exposition format.
    pub fn render(&self) -> String {
        let series = self.series.lock().expect("metrics lock").clone();
        let mut out = String::new();
        out.push_str(
            "# HELP mise_http_requests_total Requests answered, by method, route and status.\n",
        );
        out.push_str("# TYPE mise_http_requests_total counter\n");
        for ((method, route, status), s) in &series {
            let labels = labels(method, route, *status);
            let _ = writeln!(out, "mise_http_requests_total{{{labels}}} {}", s.count);
        }
        out.push_str(
            "# HELP mise_http_request_duration_seconds How long requests took to answer, by method, route and status.\n",
        );
        out.push_str("# TYPE mise_http_request_duration_seconds histogram\n");
        for ((method, route, status), s) in &series {
            let labels = labels(method, route, *status);
            let mut below = 0;
            for (bound, n) in BUCKETS.iter().zip(s.buckets) {
                below += n;
                let _ = writeln!(
                    out,
                    "mise_http_request_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {below}"
                );
            }
            let _ = writeln!(
                out,
                "mise_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}",
                s.count
            );
            let _ = writeln!(
                out,
                "mise_http_request_duration_seconds_sum{{{labels}}} {}",
                s.seconds
            );
            let _ = writeln!(
                out,
                "mise_http_request_duration_seconds_count{{{labels}}} {}",
                s.count
            );
        }
        out
    }
}

/// A series' labels, each value escaped as the format asks.
fn labels(method: &str, route: &str, status: u16) -> String {
    let escape = |v: &str| {
        v.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    };
    format!(
        "method=\"{}\",route=\"{}\",status=\"{status}\"",
        escape(method),
        escape(route)
    )
}

/// The route a request matched, as written in the router (`/api/v1/dishes/{slug}`), so one series stands for every
/// dish rather than one each; `web app` for the web app's files, and `unmatched` for an API address that is none.
fn route(req: &Request) -> String {
    if let Some(path) = req.extensions().get::<MatchedPath>() {
        return path.as_str().to_owned();
    }
    if crate::api_path(req.uri().path()).is_some() || req.uri().path().starts_with("/api/") {
        "unmatched".into()
    } else {
        "web app".into()
    }
}

/// Counts each request and how long it took to answer.
pub async fn measure(State(metrics): State<Arc<Metrics>>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let route = route(&req);
    let started = Instant::now();
    let res = next.run(req).await;
    metrics.record(
        &method,
        &route,
        res.status().as_u16(),
        started.elapsed().as_secs_f64(),
    );
    res
}

/// What the metrics port serves: `/metrics`, and nothing else.
pub fn metrics_router(metrics: Arc<Metrics>) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let metrics = metrics.clone();
            async move {
                (
                    [(
                        header::CONTENT_TYPE,
                        "text/plain; version=0.0.4; charset=utf-8",
                    )],
                    metrics.render(),
                )
                    .into_response()
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_from_a_proxy_is_kept_only_when_it_is_plain() {
        assert!(acceptable("5f2c1d8e-0b7a-4c1e-9a3d-2b6f8e1c4a7d"));
        assert!(acceptable("req_42.edge:1"));
        assert!(!acceptable(""));
        assert!(!acceptable("two words"));
        assert!(!acceptable("line\nbreak"));
        assert!(!acceptable(&"x".repeat(129)));
    }

    #[test]
    fn metrics_read_as_prometheus_text() {
        let m = Metrics::default();
        m.record("GET", "/api/v1/dishes/{slug}", 200, 0.003);
        m.record("GET", "/api/v1/dishes/{slug}", 200, 0.2);
        m.record("POST", "/api/v1/plan/entries", 400, 0.02);
        let text = m.render();
        assert!(text.contains(
            "mise_http_requests_total{method=\"GET\",route=\"/api/v1/dishes/{slug}\",status=\"200\"} 2\n"
        ));
        // Buckets count everything at or below their bound.
        assert!(text.contains("route=\"/api/v1/dishes/{slug}\",status=\"200\",le=\"0.005\"} 1\n"));
        assert!(text.contains("route=\"/api/v1/dishes/{slug}\",status=\"200\",le=\"0.1\"} 1\n"));
        assert!(text.contains("route=\"/api/v1/dishes/{slug}\",status=\"200\",le=\"0.25\"} 2\n"));
        assert!(text.contains("route=\"/api/v1/dishes/{slug}\",status=\"200\",le=\"+Inf\"} 2\n"));
        assert!(text.contains(
            "mise_http_request_duration_seconds_count{method=\"POST\",route=\"/api/v1/plan/entries\",status=\"400\"} 1\n"
        ));
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(
            labels("GET", "a\"b\\c\nd", 200),
            "method=\"GET\",route=\"a\\\"b\\\\c\\nd\",status=\"200\""
        );
    }
}
