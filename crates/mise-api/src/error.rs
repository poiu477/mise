//! API errors as JSON: `{"error": "..."}` with a matching status.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    BadRequest(String),
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
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "request failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let error = match self {
            ApiError::Internal(_) => "internal error".to_string(),
            e => e.to_string(),
        };
        (status, Json(ErrorBody { error })).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
