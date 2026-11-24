//! Importing recipes from a web page or a file. For a page, the server fetches it and its photo (from the public
//! internet only, see `mise_ingest::fetch`) and reads its schema.org Recipe data; a file is an export from another
//! recipe app, sent whole (see `mise_ingest::files`). Either way each recipe becomes a version for the household,
//! which alone sees it, and importing an unchanged page or file again changes nothing.

use axum::{
    Extension, Json,
    body::Bytes,
    extract::{FromRequest, Query, Request, State},
    http::{StatusCode, header},
};
use mise_ingest::fetch::FetchError;
use mise_ingest::files::{self, FileError};
use mise_ingest::web::{self, ImportError};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use utoipa::{IntoParams, ToSchema};
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
    /// The page's address, such as `https://recipes.example/leek-soup`.
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
        recipes: imported.into_iter().map(ImportedRecipe::from).collect(),
    }))
}

impl From<mise_ingest::import::Imported> for ImportedRecipe {
    fn from(r: mise_ingest::import::Imported) -> Self {
        ImportedRecipe {
            variant_id: r.variant_id,
            dish_slug: r.dish_slug,
            title: r.title,
            unchanged: r.unchanged,
            photo_problem: r.photo_problem,
        }
    }
}

/// An export file's bytes, as the API document describes the request body.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct ExportFile(pub Vec<u8>);

/// An export sent as a request's whole body. Its type must be an `application/` one (`application/octet-stream`
/// will do for any), which a form on another site can't send, so a signed-in visitor's browser can't be made to
/// import something.
pub struct ExportBody(pub Bytes);

impl<S: Send + Sync> FromRequest<S> for ExportBody {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim_start().to_ascii_lowercase())
            .unwrap_or_default();
        if !content_type.starts_with("application/")
            || content_type.starts_with("application/x-www-form-urlencoded")
        {
            return Err(ApiError::BadRequest(
                "send the file as the body, as application/octet-stream".into(),
            ));
        }
        Bytes::from_request(req, state)
            .await
            .map(ExportBody)
            .map_err(|e| match e.status() {
                StatusCode::PAYLOAD_TOO_LARGE => ApiError::PayloadTooLarge,
                _ => ApiError::BadRequest(e.body_text()),
            })
    }
}

/// What the file was called.
#[derive(Deserialize, IntoParams)]
pub struct FileName {
    /// The file's name, such as `My Recipes.paprikarecipes`. What a file is comes from what it holds; the name is
    /// used in messages, and tells a Nextcloud Cookbook `recipe.json` sent on its own from other schema.org JSON.
    pub name: Option<String>,
}

/// Something in the file that looked like a recipe but couldn't be read.
#[derive(Serialize, ToSchema)]
pub struct SkippedEntry {
    /// Where in the file it was: `recipes/soup/soup.json`.
    pub path: String,
    /// Why it was left out: "not valid JSON (…)", "the recipe has no name".
    pub reason: String,
}

/// What importing a file did.
#[derive(Serialize, ToSchema)]
pub struct FileImport {
    /// The apps its recipes came from, by name ("Paprika"); usually one.
    pub formats: Vec<String>,
    /// Each recipe, in the order the file lists them.
    pub recipes: Vec<ImportedRecipe>,
    /// What looked like a recipe but couldn't be read; the rest was imported.
    pub skipped: Vec<SkippedEntry>,
}

/// One file import at a time: an export may unpack to hundreds of megabytes and hundreds of photos to decode.
static FILE_IMPORTS: Semaphore = Semaphore::const_new(1);

/// Import the recipes in a file another recipe app exported: schema.org JSON, a Nextcloud Cookbook folder (zipped)
/// or `recipe.json`, a Mealie or Tandoor export zip, or a Paprika export (`.paprikarecipes`) or recipe. The body is
/// the file itself, at most 100 MB, sent as `application/octet-stream`; what it is comes from what it holds. Each
/// recipe becomes a version only this household sees, grouped into a dish like any other, with the photo the file
/// keeps for it (photos it only links to aren't fetched). Importing an unchanged file again changes nothing; a recipe
/// changed in the app since becomes a new version. Something that looked like a recipe but couldn't be read is
/// listed and the rest imported.
#[utoipa::path(
    post, path = "/api/v1/imports/file", operation_id = "import_file", tag = "recipes", params(FileName),
    request_body(content = ExportFile, content_type = "application/octet-stream", description = "The export file"),
    responses(
        (status = 200, description = "The file's recipes, imported", body = FileImport),
        (status = 400, description = "The body isn't sent as an application type, or the file holds no recipe mise reads or more than 2,000, is a zip mise can't unpack, or unpacks past the limits (300 MB, 20,000 files, 25 MB for any one)", body = crate::error::ErrorBody),
        (status = 413, description = "The file is over 100 MB", body = crate::error::ErrorBody),
        crate::error::SignedIn,
        crate::error::ReadOnlyToken
    )
)]
pub async fn create_from_file(
    State(s): State<AppState>,
    Extension(me): Extension<CurrentUser>,
    Query(file): Query<FileName>,
    ExportBody(body): ExportBody,
) -> ApiResult<Json<FileImport>> {
    let _turn = FILE_IMPORTS
        .acquire()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let name = file.name.unwrap_or_default();
    let imported = files::import_file(&s.pool, &s.media_root, me.household_id, &name, body.into())
        .await
        .map_err(|e| match e {
            FileError::Failed(e) => ApiError::Internal(e),
            FileError::Io(e) => ApiError::Internal(e.into()),
            e => ApiError::BadRequest(e.to_string()),
        })?;
    Ok(Json(FileImport {
        formats: imported
            .formats
            .iter()
            .map(|f| f.name().to_owned())
            .collect(),
        recipes: imported
            .recipes
            .into_iter()
            .map(ImportedRecipe::from)
            .collect(),
        skipped: imported
            .skipped
            .into_iter()
            .map(|s| SkippedEntry {
                path: s.path,
                reason: s.reason,
            })
            .collect(),
    }))
}
