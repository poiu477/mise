//! Importing recipes from a web page: the server fetches the page and its photo (from the public internet only,
//! see `mise_ingest::fetch`), reads its schema.org Recipe data and makes a version of each recipe for the household,
//! which alone sees it. Importing an unchanged page again changes nothing.

use axum::{Extension, Json, extract::State};
use mise_ingest::fetch::FetchError;
use mise_ingest::web::{self, ImportError};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    AppState,
    auth::CurrentUser,
    error::{ApiError, ApiResult},
    extract::JsonBody,
};

/// A page to import.
#[derive(Deserialize, ToSchema)]
pub struct ImportInput {
    /// The page's address: "https://recipes.example/leek-soup".
    pub url: String,
}

/// One recipe from the page, as imported.
#[derive(Serialize, ToSchema)]
pub struct ImportedRecipe {
    /// The version made of it: `/api/v1/variants/{id}` shows it at any serving count.
    pub variant_id: Uuid,
    /// The slug of the dish it joined, for `/api/v1/dishes/{slug}`.
    pub dish_slug: String,
    pub title: String,
    /// The household had already imported this recipe as it is now, so nothing changed.
    pub unchanged: bool,
    /// Why the recipe's photo wasn't kept, when it has one that wasn't: "its photo couldn't be kept: the site took
    /// too long to answer".
    pub photo_problem: Option<String>,
}

/// What importing a page did.
#[derive(Serialize, ToSchema)]
pub struct Import {
    /// Each recipe on the page, in page order; most pages have one.
    pub recipes: Vec<ImportedRecipe>,
}

/// Import the recipes on a web page into the household. The server fetches the page, which must be on the public
/// internet (unless the install sets `IMPORT_ALLOW_PRIVATE`), and reads its schema.org Recipe data; each recipe
/// becomes a version only this household sees, grouped into a dish like any other. Importing a page whose recipe
/// hasn't changed changes nothing; a changed one becomes a new version.
#[utoipa::path(
    post, path = "/api/v1/imports", operation_id = "import_url", tag = "recipes", request_body = ImportInput,
    responses(
        (status = 200, description = "The page's recipes, imported", body = Import),
        (status = 400, description = "The address isn't a web link mise fetches (not http or https, a private or local address, a port of another service), there's no site at it, or the page isn't a page, is too large or holds no recipe; or the body isn't JSON of the right shape", body = crate::error::ErrorBody),
        (status = 502, description = "The site didn't answer, took too long, redirected too often or answered with an error", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken,
        crate::error::TooLarge
    )
)]
pub async fn create(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    JsonBody(input): JsonBody<ImportInput>,
) -> ApiResult<Json<Import>> {
    let url = url::Url::parse(input.url.trim()).map_err(|_| {
        ApiError::BadRequest("that isn't a link: paste the page's whole address".into())
    })?;
    let imported = web::import_url(&s.pool, &s.media_root, &s.fetcher, me.household_id, &url)
        .await
        .map_err(|e| match e {
            ImportError::Failed(e) => ApiError::Internal(e),
            ImportError::Fetch(
                FetchError::Unreachable { .. }
                | FetchError::TimedOut
                | FetchError::Status(_)
                | FetchError::Redirects,
            ) => ApiError::SiteFailed(e.to_string()),
            e => ApiError::BadRequest(e.to_string()),
        })?;
    Ok(Json(Import {
        recipes: imported
            .into_iter()
            .map(|r| ImportedRecipe {
                variant_id: r.variant_id,
                dish_slug: r.dish_slug,
                title: r.title,
                unchanged: r.unchanged,
                photo_problem: r.photo_problem,
            })
            .collect(),
    }))
}
