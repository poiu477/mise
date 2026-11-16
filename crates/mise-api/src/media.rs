//! Photos: the stored original or a web-size copy, by media id.
//!
//! A photo is served to a household that can see a version using it (as its photo or a step's), and a food's
//! picture to everyone, as the food is. Any other photo answers 404, exactly as an id that was never stored, so
//! asking by id tells a household nothing about another's photos. The API hands out no other photo links: dishes
//! and the plan link a visible version's photo, a version links its own, its steps' and its ingredients' foods'.

use axum::{Extension, extract::State, http::header, response::IntoResponse};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::Path,
};

/// Web-size copies the media job makes (longest side in pixels).
pub const SIZES: [i32; 2] = [400, 1200];

/// What a photo's answer tells caches: the browser that asked may keep it for a year, and nothing shared may.
pub const CACHE_CONTROL: &str = "private, max-age=31536000, immutable";

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
            small: format!("/api/v1/media/{id}/400"),
            large: format!("/api/v1/media/{id}/1200"),
            original: format!("/api/v1/media/{id}/original"),
        }
    }
}

/// Where photo `$1` is stored and its type, at web size `$3` when that copy exists and as the original otherwise
/// (`$3` null asks for the original); no row unless household `$2` may see the photo. Photos are stored once by
/// content, so one photo can be several versions': it is enough that the household can see one of them
/// (`visible_to`, the rule every recipe query uses). A food's picture comes only from versions no household owns
/// (`mise-ingest`'s media pass), so it is everyone's.
const VISIBLE_PHOTO: &str = "
    SELECT coalesce(d.path, m.path), coalesce(d.mime, m.mime)
    FROM media m LEFT JOIN media_derivatives d ON d.media_id = m.id AND d.max_side = $3
    WHERE m.id = $1
      AND (EXISTS (SELECT 1 FROM variant_media vm JOIN variants v ON v.id = vm.variant_id
                   WHERE vm.media_id = m.id AND visible_to(v, $2))
           OR EXISTS (SELECT 1 FROM foods f WHERE f.props->>'image_media_id' = m.id::text))";

/// A photo. `size` is `400`, `1200` or `original`; a size the photo was never made in falls back to the original.
/// A photo is served only to a household that can see a version using it; a food's picture is served to everyone.
/// Another household's photo is not found, as if it didn't exist.
#[utoipa::path(
    get, path = "/api/v1/media/{id}/{size}", operation_id = "get_media", tag = "media",
    params(("id" = Uuid, Path), ("size" = String, Path, description = "400, 1200 or original")),
    responses(
        (status = 200, description = "The image"),
        (status = 400, description = "The id isn't a UUID, or the size isn't 400, 1200 or original", body = crate::error::ErrorBody),
        (status = 404, description = "No such photo, not one a version this household can see uses, or its file is missing", body = crate::error::ErrorBody),
        crate::error::SignedIn
    )
)]
pub async fn get(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Path((id, size)): Path<(Uuid, String)>,
) -> ApiResult<impl IntoResponse> {
    let max_side: Option<i32> = match size.as_str() {
        "original" => None,
        other => Some(
            other
                .parse()
                .ok()
                .filter(|n| SIZES.contains(n))
                .ok_or_else(|| ApiError::BadRequest("size must be 400, 1200 or original".into()))?,
        ),
    };
    let (path, mime): (String, String) = sqlx::query_as(VISIBLE_PHOTO)
        .bind(id)
        .bind(me.household_id)
        .bind(max_side)
        .fetch_optional(&s.pool)
        .await?
        .ok_or(ApiError::NotFound)?;
    let bytes = tokio::fs::read(s.media_root.join(&path))
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ApiError::NotFound
            } else {
                ApiError::Internal(e.into())
            }
        })?;
    // Content-addressed: a media id never changes its bytes, so a browser keeps them for good. `private`, because
    // who may see a photo depends on who asks: a cache shared between people (a proxy, a CDN) must not keep it and
    // answer for the server.
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, CACHE_CONTROL.into()),
        ],
        bytes,
    ))
}
