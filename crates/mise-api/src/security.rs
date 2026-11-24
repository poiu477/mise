//! What the server does on every response and request to keep the app safe in a browser: response headers that say
//! where the app may load from and that no other site may frame it, and a check that refuses a request to change
//! something when a browser says another site sent it.
//!
//! The request check is the one Go's `net/http` (`CrossOriginProtection`) makes, and needs no token. A browser
//! labels each request with `Sec-Fetch-Site`; a change asked for by another site, or by a sibling on the same domain
//! (`cross-site`, `same-site`), is refused. A browser too old to send it sends `Origin` with every POST, PUT, PATCH
//! and DELETE, which must then be this server's own: the host the request was sent to, or `PUBLIC_BASE_URL`. A
//! request with neither header doesn't come from a browser page (the `mise` command, a script with an API token),
//! and the session cookie, the only thing another site could make a browser spend, isn't in play.
//!
//! See "Requests from other sites" in docs/architecture.md for how this fits with the session cookie's `SameSite`
//! and the content types each route takes.

use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use url::Url;

use crate::{AppState, error::ApiError};

/// What a page of mise may load, and from where: its own scripts, styles, photos (and the editor's preview of a
/// photo not yet sent, a `blob:`), and nothing from anywhere else. No inline script or style runs, no plugin loads,
/// no form posts elsewhere, and no other site may frame it.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     img-src 'self' blob: data:; connect-src 'self'; font-src 'self'; manifest-src 'self'; worker-src 'self'; \
     media-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'";

/// The headers every response carries, unless a route sets its own.
pub fn headers() -> [(header::HeaderName, HeaderValue); 4] {
    [
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ),
        // A response is only ever what its Content-Type says: a photo is never run as a script.
        (
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ),
        // Addresses within mise go with a request to mise, and to no other site: a dish's address, a kitchen link.
        (
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ),
        // frame-ancestors, for browsers that don't read it.
        (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
    ]
}

/// Whether a request asks to change something: anything but reading.
fn changes(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Whether a browser sent this request from another site, as far as its headers tell.
pub fn from_another_site(req: &Request, public: Option<&Url>) -> bool {
    let headers = req.headers();
    if let Some(site) = headers.get("sec-fetch-site") {
        // `none` is the person's own doing: a bookmark, an address typed in.
        return !matches!(site.as_bytes(), b"same-origin" | b"none");
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    let Some(origin) = origin.to_str().ok().and_then(|o| Url::parse(o).ok()) else {
        // `Origin: null` (a sandboxed frame, a privacy-minded redirect) can't be this server.
        return true;
    };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    let ours = |url: &Url| {
        let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        url.host_str().map(|h| format!("{h}{port}"))
    };
    let sent_to_host = ours(&origin).is_some_and(|o| Some(o.as_str()) == host);
    let public_origin = public.is_some_and(|p| p.origin() == origin.origin());
    !(sent_to_host || public_origin)
}

/// Refuses a request to change something that a browser sent from another site, before anything else reads it.
pub async fn refuse_cross_site(State(s): State<AppState>, req: Request, next: Next) -> Response {
    if changes(req.method()) && from_another_site(&req, s.auth.config.public_base_url.as_ref()) {
        return ApiError::Forbidden(
            "this request came from another site, so mise refused it".into(),
        )
        .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn request(method: Method, headers: &[(&str, &str)]) -> Request {
        let mut b = Request::builder().method(method).uri("/api/v1/auth/logout");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    #[test]
    fn a_browser_s_own_label_decides() {
        for (site, refused) in [
            ("same-origin", false),
            ("none", false),
            ("same-site", true),
            ("cross-site", true),
        ] {
            let req = request(Method::POST, &[("sec-fetch-site", site)]);
            assert_eq!(from_another_site(&req, None), refused, "{site}");
        }
    }

    #[test]
    fn without_the_label_the_origin_must_be_this_server() {
        let host = ("host", "mise.home.example:8080");
        let same = request(
            Method::POST,
            &[host, ("origin", "http://mise.home.example:8080")],
        );
        assert!(!from_another_site(&same, None));
        let other = request(Method::POST, &[host, ("origin", "https://evil.example")]);
        assert!(from_another_site(&other, None));
        let null = request(Method::POST, &[host, ("origin", "null")]);
        assert!(from_another_site(&null, None));

        // Behind a proxy that sends its own Host, the public address counts as this server's.
        let proxied = request(
            Method::POST,
            &[
                ("host", "127.0.0.1:8080"),
                ("origin", "https://mise.example.com"),
            ],
        );
        let public = Url::parse("https://mise.example.com/").unwrap();
        assert!(!from_another_site(&proxied, Some(&public)));
        assert!(from_another_site(&proxied, None));
    }

    #[test]
    fn a_request_from_no_browser_page_is_left_alone() {
        let req = request(Method::DELETE, &[("host", "mise.home.example")]);
        assert!(!from_another_site(&req, None));
    }

    #[test]
    fn reading_is_never_refused() {
        assert!(!changes(&Method::GET) && !changes(&Method::HEAD) && !changes(&Method::OPTIONS));
        assert!(changes(&Method::POST) && changes(&Method::DELETE) && changes(&Method::PATCH));
    }
}
