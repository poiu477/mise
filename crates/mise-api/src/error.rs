//! API errors as JSON: `{"error": "...", "code": "..."}` with a matching status. `error` is a sentence for people;
//! `code` is a stable name for programs, so a client never has to match on status or wording.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// No such thing, or no route at this address under the API.
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
    #[error("sign in first")]
    Unauthorized,
    #[error("wrong username or password")]
    BadCredentials,
    #[error("{0}")]
    Forbidden(String),
    /// The route exists but doesn't take this method (axum's 405, which has no body of its own).
    #[error("this address doesn't take that method")]
    MethodNotAllowed,
    #[error("{0}")]
    Conflict(String),
    /// The request was made for another account than the one now signed in (see `ACCOUNT_HEADER`).
    #[error("signed in as a different account")]
    AccountChanged,
    /// The request body is over the size the server reads (axum's limit, 2 MB).
    #[error("that's too much to send at once")]
    PayloadTooLarge,
    /// Too many attempts; try again after this many seconds.
    #[error("too many attempts; try again in {0} seconds")]
    TooManyRequests(u64),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(e.into())
    }
}

/// What kind of error a response is, named for programs: stable across releases, whatever the message says.
#[derive(Serialize, utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 404: no such thing, or nothing at this address.
    NotFound,
    /// 400: the request is malformed or its values are refused (bad JSON, path, query or field).
    BadRequest,
    /// 401: no session or token, or one that has ended.
    Unauthorized,
    /// 401: signing in with a wrong username or password.
    BadCredentials,
    /// 403: signed in, but not allowed to do this.
    Forbidden,
    /// 405: the address doesn't take this method.
    MethodNotAllowed,
    /// 409: clashes with what is already there.
    Conflict,
    /// 412: made for another account than the one signed in now (`X-Mise-Account`).
    AccountChanged,
    /// 413: the request body is too large.
    PayloadTooLarge,
    /// 429: too many attempts; `Retry-After` says when to try again.
    TooManyRequests,
    /// 500: the server failed; the details are in its log.
    Internal,
}

impl ApiError {
    /// The status this error answers with.
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized | ApiError::BadCredentials => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::AccountChanged => StatusCode::PRECONDITION_FAILED,
            ApiError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The stable name of this kind of error.
    pub fn code(&self) -> ErrorCode {
        match self {
            ApiError::NotFound => ErrorCode::NotFound,
            ApiError::BadRequest(_) => ErrorCode::BadRequest,
            ApiError::Unauthorized => ErrorCode::Unauthorized,
            ApiError::BadCredentials => ErrorCode::BadCredentials,
            ApiError::Forbidden(_) => ErrorCode::Forbidden,
            ApiError::MethodNotAllowed => ErrorCode::MethodNotAllowed,
            ApiError::Conflict(_) => ErrorCode::Conflict,
            ApiError::AccountChanged => ErrorCode::AccountChanged,
            ApiError::PayloadTooLarge => ErrorCode::PayloadTooLarge,
            ApiError::TooManyRequests(_) => ErrorCode::TooManyRequests,
            ApiError::Internal(_) => ErrorCode::Internal,
        }
    }
}

/// The body of every error the API answers with.
#[derive(Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    /// What went wrong, as a sentence to show people: "servings must be more than 0 and at most 24".
    pub error: String,
    /// What kind of error it is, for programs to act on.
    pub code: ErrorCode,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = (self.status(), self.code());
        if let ApiError::Internal(e) = &self {
            tracing::error!(error = ?e, "request failed");
        }
        let retry = match &self {
            ApiError::TooManyRequests(s) => HeaderValue::from_str(&s.to_string()).ok(),
            _ => None,
        };
        let error = match self {
            ApiError::Internal(_) => "internal error".to_string(),
            e => e.to_string(),
        };
        let mut res = (status, Json(ErrorBody { error, code })).into_response();
        if let Some(r) = retry {
            res.headers_mut().insert(header::RETRY_AFTER, r);
        }
        res
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

// Responses many routes share, written once and named in each route's `#[utoipa::path]` beside its own:
// `responses((status = 200, ...), SignedIn, ReadOnlyToken)`. Each answers before the handler runs, or apart from
// it, so no handler's code shows them. tests/responses.rs checks every route gives each response it documents and
// none it doesn't.

/// Error responses with the error body, from (status, description) pairs.
fn documented(list: &[(u16, &str)]) -> ResponseMap {
    use utoipa::openapi::{ContentBuilder, Ref, ResponseBuilder};
    list.iter()
        .map(|&(status, description)| {
            let body = ContentBuilder::new()
                .schema(Some(Ref::from_schema_name("ErrorBody")))
                .build();
            let response = ResponseBuilder::new()
                .description(description)
                .content("application/json", body)
                .build();
            (status.to_string(), response.into())
        })
        .collect()
}

type ResponseMap =
    std::collections::BTreeMap<String, utoipa::openapi::RefOr<utoipa::openapi::response::Response>>;

const SERVER_FAILED: (u16, &str) = (500, "The server failed; the details are in its log");

/// A server failure: any route that reads or writes the database.
pub struct ServerError;

impl utoipa::IntoResponses for ServerError {
    fn responses() -> ResponseMap {
        documented(&[SERVER_FAILED])
    }
}

/// What every route that needs a signed-in person can answer before its handler runs (`auth::require_auth`), and a
/// server failure.
pub struct SignedIn;

impl utoipa::IntoResponses for SignedIn {
    fn responses() -> ResponseMap {
        documented(&[
            (
                401,
                "Not signed in: no session or token, or one that has ended",
            ),
            (
                412,
                "`X-Mise-Account` names another account than the one signed in",
            ),
            SERVER_FAILED,
        ])
    }
}

/// A change refused to a read-only API token or a kitchen device (`auth::require_auth`). Routes that refuse other
/// callers too write their own 403 instead.
pub struct ReadOnlyToken;

impl utoipa::IntoResponses for ReadOnlyToken {
    fn responses() -> ResponseMap {
        documented(&[(
            403,
            "Called with a read-only API token or from a kitchen device, neither of which can make changes",
        )])
    }
}

/// A read refused to a kitchen device (`auth::require_auth`), which reads only the plan and recipes: for routes
/// that refuse nobody else. Routes that refuse other callers too name kitchen devices in their own 403.
pub struct KitchenDevice;

impl utoipa::IntoResponses for KitchenDevice {
    fn responses() -> ResponseMap {
        documented(&[(
            403,
            "Called from a kitchen device, which reads only the plan and recipes",
        )])
    }
}

/// A JSON body over the size the server reads (2 MB; `extract::JsonBody`).
pub struct TooLarge;

impl utoipa::IntoResponses for TooLarge {
    fn responses() -> ResponseMap {
        documented(&[(413, "The body is over 2 MB")])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status and body an error answers with.
    async fn answer(e: ApiError) -> (StatusCode, serde_json::Value) {
        let res = e.into_response();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn every_error_says_what_happened_and_names_its_kind() {
        let cases = [
            (ApiError::NotFound, 404, "not_found"),
            (
                ApiError::BadRequest("give it a name".into()),
                400,
                "bad_request",
            ),
            (ApiError::Unauthorized, 401, "unauthorized"),
            (ApiError::BadCredentials, 401, "bad_credentials"),
            (ApiError::Forbidden("admins only".into()), 403, "forbidden"),
            (ApiError::MethodNotAllowed, 405, "method_not_allowed"),
            (ApiError::Conflict("taken".into()), 409, "conflict"),
            (ApiError::AccountChanged, 412, "account_changed"),
            (ApiError::PayloadTooLarge, 413, "payload_too_large"),
            (ApiError::TooManyRequests(30), 429, "too_many_requests"),
            (anyhow::anyhow!("disk on fire").into(), 500, "internal"),
        ];
        for (e, status, code) in cases {
            let (s, body) = answer(e).await;
            assert_eq!(s.as_u16(), status, "{body}");
            assert_eq!(body["code"], code, "{body}");
            assert!(
                body["error"].as_str().is_some_and(|m| !m.is_empty()),
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn a_server_failure_keeps_its_details_to_the_log() {
        let (_, body) = answer(anyhow::anyhow!("password=hunter2 in the query").into()).await;
        assert_eq!(body["error"], "internal error");
    }
}
