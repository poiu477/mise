//! Photos: the stored original or a web-size copy, by media id.

use axum::{
    extract::{Path, State},
    http::header,
    response::IntoResponse,
};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
};

/// Web-size copies the media job makes (longest side in pixels).
pub const SIZES: [i32; 2] = [400, 1200];

/// Links to one photo in every size, for responses.
#[derive(Serialize, utoipa::ToSchema, Clone, Debug)]
pub struct Image {
    pub id: Uuid,
    /// Longest side at most 400 px.
    pub small: String,
    /// Longest side at most 1200 px.
    pub large: String,
    pub original: String,
}

impl Image {
    pub fn new(id: Uuid) -> Self {
        Image {
            id,
            small: format!("/api/media/{id}/400"),
            large: format!("/api/media/{id}/1200"),
            original: format!("/api/media/{id}/original"),
        }
    }
}

/// A photo. `size` is `400`, `1200` or `original`; a size the photo was never made in falls back to the original.
#[utoipa::path(
    get, path = "/api/media/{id}/{size}", tag = "media",
    params(("id" = Uuid, Path), ("size" = String, Path, description = "400, 1200 or original")),
    responses((status = 200, description = "The image"), (status = 404, body = crate::error::ErrorBody))
)]
pub async fn get(State(s): State<AppState>, Path((id, size)): Path<(Uuid, String)>) -> ApiResult<impl IntoResponse> {
    let max_side: Option<i32> = match size.as_str() {
        "original" => None,
        other => Some(other.parse().ok().filter(|n| SIZES.contains(n)).ok_or_else(|| ApiError::BadRequest("size must be 400, 1200 or original".into()))?),
    };
    let derivative: Option<(String, String)> = match max_side {
        Some(n) => sqlx::query_as("SELECT path, mime FROM media_derivatives WHERE media_id = $1 AND max_side = $2")
            .bind(id)
            .bind(n)
            .fetch_optional(&s.pool)
            .await?,
        None => None,
    };
    let (path, mime) = match derivative {
        Some(d) => d,
        None => sqlx::query_as("SELECT path, mime FROM media WHERE id = $1").bind(id).fetch_optional(&s.pool).await?.ok_or(ApiError::NotFound)?,
    };
    let bytes = tokio::fs::read(s.media_root.join(&path)).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { ApiError::NotFound } else { ApiError::Internal(e.into()) }
    })?;
    // Content-addressed: a media id never changes its bytes.
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=31536000, immutable".into())], bytes))
}
