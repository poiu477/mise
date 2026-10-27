//! API errors as JSON: `{"error": "..."}` with a matching status.

use axum::{
    Json,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
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
    #[error("{0}")]
    Conflict(String),
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

#[derive(Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    pub error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized | ApiError::BadCredentials => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "request failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let retry = match &self {
            ApiError::TooManyRequests(s) => HeaderValue::from_str(&s.to_string()).ok(),
            _ => None,
        };
        let error = match self {
            ApiError::Internal(_) => "internal error".to_string(),
            e => e.to_string(),
        };
        let mut res = (status, Json(ErrorBody { error })).into_response();
        if let Some(r) = retry {
            res.headers_mut().insert(header::RETRY_AFTER, r);
        }
        res
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
