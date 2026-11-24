//! Import recipes from a web page into a household: fetch the page ([`crate::fetch`]), read its schema.org Recipe
//! data ([`crate::schema_org`]) and store each recipe as a record of the `web` source ([`crate::import`]), which makes
//! a version of it that the importing household alone sees and runs the shared stages over it.
//!
//! Importing a page whose recipe hasn't changed changes nothing; one that has changed becomes a new version whose
//! parent is the previous one.

use std::path::Path;

use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::fetch::{FetchError, Fetcher};
pub use crate::import::Imported;
use crate::import::{self, Source, Stored};
use crate::schema_org::{self, ReadError};

/// The source every web page's records belong to.
pub const SOURCE: Source = Source {
    key: "web",
    name: "Web pages",
    kind: "web",
};

/// The most recipes taken from one page: a page with more is a list of them, not a recipe.
pub const MAX_RECIPES: usize = 10;

/// Why a page wasn't imported.
#[derive(Debug)]
pub enum ImportError {
    /// The page couldn't be fetched, or the address was refused.
    Fetch(FetchError),
    /// The page was fetched but holds no recipe mise can read.
    Read(ReadError),
    /// The page holds more than [`MAX_RECIPES`] recipes.
    TooManyRecipes(usize),
    /// Storing it failed: the database or the disk.
    Failed(anyhow::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Fetch(e) => write!(f, "couldn't import that link: {e}"),
            ImportError::Read(e) => write!(f, "couldn't import that link: {e}"),
            ImportError::TooManyRecipes(n) => write!(
                f,
                "couldn't import that link: the page lists {n} recipes, more than the {MAX_RECIPES} mise takes from one page"
            ),
            ImportError::Failed(e) => write!(f, "importing failed: {e:#}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<anyhow::Error> for ImportError {
    fn from(e: anyhow::Error) -> Self {
        ImportError::Failed(e)
    }
}

impl From<sqlx::Error> for ImportError {
    fn from(e: sqlx::Error) -> Self {
        ImportError::Failed(e.into())
    }
}

/// Fetches the page at `url` and imports every recipe on it into `household`, photos stored under `media_root`.
/// Answers with each recipe in page order.
pub async fn import_url(
    pool: &PgPool,
    media_root: &Path,
    fetcher: &Fetcher,
    household: Uuid,
    url: &Url,
) -> Result<Vec<Imported>, ImportError> {
    let page = fetcher.page(url).await.map_err(ImportError::Fetch)?;
    let records = schema_org::read_page(&page.html, Some(&page.url)).map_err(ImportError::Read)?;
    if records.len() > MAX_RECIPES {
        return Err(ImportError::TooManyRecipes(records.len()));
    }

    let mut stored = Vec::new();
    for record in &records {
        let hash = import::content_hash(record, None);
        let (variant_id, fresh) = import::store(pool, &SOURCE, household, record, &hash).await?;
        let mut photo_problem = None;
        if fresh && let Some(hero) = &record.hero_image {
            photo_problem = photo(pool, media_root, fetcher, variant_id, hero)
                .await?
                .map(|why| format!("its photo couldn't be kept: {why}"));
        }
        stored.push(Stored {
            variant_id,
            title: record.title.clone(),
            unchanged: !fresh,
            photo_problem,
        });
    }
    Ok(import::finish(pool, stored).await?)
}

/// Fetches the recipe's photo through the same checks as the page and makes it the version's. Answers with why it
/// wasn't kept, when it wasn't.
async fn photo(
    pool: &PgPool,
    media_root: &Path,
    fetcher: &Fetcher,
    variant: Uuid,
    hero: &str,
) -> anyhow::Result<Option<String>> {
    let Ok(url) = Url::parse(hero) else {
        return Ok(Some("its address isn't a link".into()));
    };
    match fetcher.image(&url).await {
        Ok(bytes) => import::attach_photo(pool, media_root, variant, bytes).await,
        Err(e) => Ok(Some(e.to_string())),
    }
}
