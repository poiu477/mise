//! Import recipes from the files other recipe apps export: schema.org JSON, Nextcloud Cookbook (a folder of
//! `recipe.json` files, or the folder zipped), Mealie and Tandoor exports (zips), and Paprika exports
//! (`.paprikarecipes`). Each format's recipes are read into the records a web page gives ([`crate::schema_org`]), and
//! stored as a link import's are ([`crate::import`]): a version of each recipe that the importing household alone
//! sees, grouped into a dish, the shared stages run once for the whole file.
//!
//! What a file is comes from what it holds, not its name: a zip (looked through, a zip inside it too, as Tandoor
//! nests them), a gzip (a single Paprika recipe), or JSON, sorted into a format by its fields. A folder is read as a
//! zip of it would be. A photo is taken only from the file: Paprika's inline photo, or the image a Nextcloud, Mealie
//! or Tandoor export keeps beside the recipe. Links to photos elsewhere are kept with the recipe but not fetched.
//!
//! Each source is one per format, and a recipe's id within it is the app's own when it has one (Paprika's `uid`,
//! Mealie's `id`, Nextcloud's `id`), else its link, else a hash of it. Importing an unchanged export again changes
//! nothing; a recipe changed in the app since becomes a new version of the one imported before.

mod mealie;
mod nextcloud;
mod paprika;
mod tandoor;

use std::collections::HashMap;
use std::fmt;
use std::io::{Cursor, Read};
use std::path::Path;

use serde_json::{Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::import::{self, Imported, Source, Stored};
use crate::schema_org::{self, ReadError, Record};

/// The largest file taken whole: an export of a few hundred recipes with their photos.
pub const MAX_FILE_BYTES: usize = 100 * 1024 * 1024;
/// The most a file or folder may unpack to, all its entries together.
const MAX_UNPACKED_BYTES: u64 = 300 * 1024 * 1024;
/// The largest one entry may unpack to: a recipe, or a photo as large as an upload may be.
const MAX_ENTRY_BYTES: u64 = 25 * 1024 * 1024;
/// The most entries looked at in one file or folder.
const MAX_ENTRIES: usize = 20_000;
/// How deep zips nest: a zip, and zips inside it (Tandoor), but no further.
const MAX_NESTING: usize = 2;
/// The most recipes taken from one file.
pub const MAX_RECIPES: usize = 2_000;

/// What an export came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Format {
    /// A schema.org Recipe as JSON-LD, as a web page holds it.
    SchemaOrg,
    /// Nextcloud Cookbook: a folder of recipes, each a `recipe.json` (schema.org) with `full.jpg` beside it.
    NextcloudCookbook,
    /// Mealie: `recipes/<slug>/<slug>.json`, with `images/original.webp` beside it.
    Mealie,
    /// Tandoor: a zip of zips, each a `recipe.json` and an `image.*`.
    Tandoor,
    /// Paprika: `.paprikarecipes`, a zip of gzipped JSON recipes with their photos inline.
    Paprika,
}

impl Format {
    /// The format's name as people know it.
    pub fn name(self) -> &'static str {
        match self {
            Format::SchemaOrg => "schema.org JSON",
            Format::NextcloudCookbook => "Nextcloud Cookbook",
            Format::Mealie => "Mealie",
            Format::Tandoor => "Tandoor",
            Format::Paprika => "Paprika",
        }
    }

    /// The source its records belong to.
    pub fn source(self) -> Source {
        let (key, name) = match self {
            Format::SchemaOrg => ("file:schema-org", "schema.org JSON files"),
            Format::NextcloudCookbook => ("file:nextcloud-cookbook", "Nextcloud Cookbook exports"),
            Format::Mealie => ("file:mealie", "Mealie exports"),
            Format::Tandoor => ("file:tandoor", "Tandoor exports"),
            Format::Paprika => ("file:paprika", "Paprika exports"),
        };
        Source {
            key,
            name,
            kind: "file",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a file wasn't imported.
#[derive(Debug)]
pub enum FileError {
    /// It is larger than [`MAX_FILE_BYTES`], or unpacks past the limits; says which.
    TooLarge(String),
    /// It is a zip that can't be read: damaged, encrypted, or packed in a way mise doesn't unpack.
    Unreadable(String),
    /// Nothing in it is a recipe mise reads. Says why the likeliest candidate wasn't, when there was one.
    NoRecipes(Option<String>),
    /// It holds more than [`MAX_RECIPES`] recipes.
    TooManyRecipes(usize),
    /// A folder couldn't be read.
    Io(std::io::Error),
    /// Storing it failed: the database or the disk.
    Failed(anyhow::Error),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::TooLarge(why) => write!(f, "couldn't import that file: {why}"),
            FileError::Unreadable(why) => write!(f, "couldn't import that file: {why}"),
            FileError::NoRecipes(why) => {
                write!(
                    f,
                    "couldn't import that file: it holds no recipe mise reads (schema.org JSON, or an export \
                     from Nextcloud Cookbook, Mealie, Tandoor or Paprika)"
                )?;
                match why {
                    Some(why) => write!(f, "; {why}"),
                    None => Ok(()),
                }
            }
            FileError::TooManyRecipes(n) => write!(
                f,
                "couldn't import that file: it holds {n} recipes, more than the {MAX_RECIPES} mise takes from one file"
            ),
            FileError::Io(e) => write!(f, "couldn't read the folder: {e}"),
            FileError::Failed(e) => write!(f, "importing failed: {e:#}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<anyhow::Error> for FileError {
    fn from(e: anyhow::Error) -> Self {
        FileError::Failed(e)
    }
}

/// One file inside an export: its path within it ("recipes/soup/soup.json"; inside a nested zip, "1.zip/recipe.json")
/// and its bytes.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The path within the export, `/` between folders.
    pub path: String,
    /// What it holds.
    pub bytes: Vec<u8>,
}

/// A recipe read from an export, ready to store.
#[derive(Debug, Clone)]
pub struct Found {
    /// The app it came from.
    pub format: Format,
    /// Where in the export it was.
    pub path: String,
    /// The recipe.
    pub record: Record,
    /// Its photo, and where in the export that was ("…/full.jpg", or "photo_data" inside a Paprika recipe).
    pub photo: Option<(String, Vec<u8>)>,
    /// Why a photo the recipe has wasn't usable, when it wasn't.
    pub photo_problem: Option<String>,
}

/// Something in an export that looked like a recipe but couldn't be read as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Where in the export it was.
    pub path: String,
    /// Why it was left out ("not valid JSON", "the recipe has no name").
    pub reason: String,
}

/// What an export holds.
#[derive(Debug, Clone, Default)]
pub struct Contents {
    /// Its recipes, in path order.
    pub recipes: Vec<Found>,
    /// What looked like a recipe but couldn't be read.
    pub skipped: Vec<Skipped>,
}

/// What importing a file did.
#[derive(Debug, Clone)]
pub struct FileImport {
    /// The formats its recipes came in, usually one.
    pub formats: Vec<Format>,
    /// Each recipe, in the order the export lists them.
    pub recipes: Vec<Imported>,
    /// What looked like a recipe but couldn't be read.
    pub skipped: Vec<Skipped>,
}

/// Imports every recipe in a file (`name` is what it was called, for messages and for telling a Nextcloud
/// `recipe.json` apart) into `household`, photos stored under `media_root`.
pub async fn import_file(
    pool: &PgPool,
    media_root: &Path,
    household: Uuid,
    name: &str,
    bytes: Vec<u8>,
) -> Result<FileImport, FileError> {
    let name = name.to_owned();
    let contents = unpacked(move || read(&unpack(&name, bytes)?)).await?;
    import(pool, media_root, household, contents).await
}

/// Unpacks and reads on a thread of its own: unzipping and parsing a large export takes a while.
async fn unpacked(
    work: impl FnOnce() -> Result<Contents, FileError> + Send + 'static,
) -> Result<Contents, FileError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| FileError::Failed(e.into()))?
}

/// Imports every recipe in a file or folder on disk, as [`import_file`] does.
pub async fn import_path(
    pool: &PgPool,
    media_root: &Path,
    household: Uuid,
    path: &Path,
) -> Result<FileImport, FileError> {
    let path = path.to_path_buf();
    let contents = unpacked(move || {
        let entries = if path.is_dir() {
            unpack_dir(&path)?
        } else {
            let size = std::fs::metadata(&path).map_err(FileError::Io)?.len();
            if size > MAX_FILE_BYTES as u64 {
                return Err(too_large_file());
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            unpack(&name, std::fs::read(&path).map_err(FileError::Io)?)?
        };
        read(&entries)
    })
    .await?;
    import(pool, media_root, household, contents).await
}

/// Stores each recipe read and runs the shared stages once.
pub async fn import(
    pool: &PgPool,
    media_root: &Path,
    household: Uuid,
    contents: Contents,
) -> Result<FileImport, FileError> {
    let mut formats: Vec<Format> = contents.recipes.iter().map(|f| f.format).collect();
    formats.sort();
    formats.dedup();
    let mut stored = Vec::with_capacity(contents.recipes.len());
    for found in contents.recipes {
        let photo = found.photo.map(|(_, bytes)| bytes);
        let hash = import::content_hash(&found.record, photo.as_deref());
        let (variant_id, fresh) = import::store(
            pool,
            &found.format.source(),
            household,
            &found.record,
            &hash,
        )
        .await?;
        let mut photo_problem = None;
        if fresh {
            photo_problem = match photo {
                Some(bytes) => import::attach_photo(pool, media_root, variant_id, bytes).await?,
                None => found.photo_problem,
            }
            .map(|why| format!("its photo couldn't be kept: {why}"));
        }
        stored.push(Stored {
            variant_id,
            title: found.record.title,
            unchanged: !fresh,
            photo_problem,
        });
    }
    Ok(FileImport {
        formats,
        recipes: import::finish(pool, stored).await?,
        skipped: contents.skipped,
    })
}

fn too_large_file() -> FileError {
    FileError::TooLarge(format!(
        "it is larger than {} MB",
        MAX_FILE_BYTES / (1024 * 1024)
    ))
}

// ---------------------------------------------------------------------------------------------------------
// Unpacking

/// What unpacking has taken so far, against the limits.
#[derive(Default)]
struct Budget {
    entries: usize,
    bytes: u64,
}

impl Budget {
    fn take(&mut self, bytes: u64) -> Result<(), FileError> {
        self.entries += 1;
        self.bytes += bytes;
        if self.entries > MAX_ENTRIES {
            return Err(FileError::TooLarge(format!(
                "it holds more than {MAX_ENTRIES} files"
            )));
        }
        if self.bytes > MAX_UNPACKED_BYTES {
            return Err(FileError::TooLarge(format!(
                "it unpacks to more than {} MB",
                MAX_UNPACKED_BYTES / (1024 * 1024)
            )));
        }
        Ok(())
    }
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// The lower-cased extension of a path's last part.
fn extension(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

/// Whether an entry inside an export is worth reading: a recipe, a photo, or a zip that may hold them. A Mac's
/// resource forks (`__MACOSX/`, `._name`) never are.
fn wanted(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    if path.split('/').any(|part| part == "__MACOSX") || file.starts_with("._") {
        return false;
    }
    let ext = extension(path);
    matches!(
        ext.as_str(),
        "json" | "paprikarecipe" | "zip" | "paprikarecipes"
    ) || IMAGE_EXTENSIONS.contains(&ext.as_str())
}

/// The entries of a file: those of a zip (and of zips inside it), or the file itself as its one entry.
pub fn unpack(name: &str, bytes: Vec<u8>) -> Result<Vec<Entry>, FileError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(too_large_file());
    }
    let mut budget = Budget::default();
    let mut out = Vec::new();
    if is_zip(&bytes) {
        unzip(name, "", bytes, 1, &mut budget, &mut out)?;
    } else if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(FileError::TooLarge(format!(
            "it is larger than {} MB, more than one recipe file may be",
            MAX_ENTRY_BYTES / (1024 * 1024)
        )));
    } else {
        budget.take(bytes.len() as u64)?;
        out.push(Entry {
            path: name.to_owned(),
            bytes,
        });
    }
    Ok(out)
}

/// Adds a zip's wanted entries to `out`, each path after `prefix`; zips inside it are opened too, `depth` deep.
fn unzip(
    name: &str,
    prefix: &str,
    bytes: Vec<u8>,
    depth: usize,
    budget: &mut Budget,
    out: &mut Vec<Entry>,
) -> Result<(), FileError> {
    let unreadable = |e: zip::result::ZipError| {
        FileError::Unreadable(format!("{name} isn't a zip mise can unpack ({e})"))
    };
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
    if archive.len() > MAX_ENTRIES {
        return Err(FileError::TooLarge(format!(
            "it holds more than {MAX_ENTRIES} files"
        )));
    }
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(unreadable)?;
        if file.is_dir() {
            continue;
        }
        let path = format!(
            "{prefix}{}",
            file.name().map_err(unreadable)?.replace('\\', "/")
        );
        if !wanted(&path) {
            continue;
        }
        if file.encrypted() {
            return Err(FileError::Unreadable(format!(
                "{path} is encrypted: export it again without a password"
            )));
        }
        // Read no more than the limit, whatever the zip says the size is.
        let mut data = Vec::new();
        (&mut file)
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| FileError::Unreadable(format!("{path} can't be unpacked ({e})")))?;
        if data.len() as u64 > MAX_ENTRY_BYTES {
            return Err(FileError::TooLarge(format!(
                "{path} unpacks to more than {} MB",
                MAX_ENTRY_BYTES / (1024 * 1024)
            )));
        }
        budget.take(data.len() as u64)?;
        if is_zip(&data) && depth < MAX_NESTING {
            unzip(&path, &format!("{path}/"), data, depth + 1, budget, out)?;
        } else {
            out.push(Entry { path, bytes: data });
        }
    }
    Ok(())
}

/// The entries of a folder and everything under it, as a zip of it would hold them. Links are not followed.
pub fn unpack_dir(dir: &Path) -> Result<Vec<Entry>, FileError> {
    let mut budget = Budget::default();
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&folder)
            .map_err(FileError::Io)?
            .collect::<Result<_, _>>()
            .map_err(FileError::Io)?;
        children.sort_by_key(|c| c.file_name());
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            let path = format!("{prefix}{name}");
            let kind = child.file_type().map_err(FileError::Io)?;
            if kind.is_dir() {
                stack.push((child.path(), format!("{path}/")));
                continue;
            }
            if !kind.is_file() || !wanted(&path) {
                continue;
            }
            let size = child.metadata().map_err(FileError::Io)?.len();
            if size > MAX_FILE_BYTES as u64 {
                return Err(FileError::TooLarge(format!(
                    "{path} is larger than {} MB",
                    MAX_FILE_BYTES / (1024 * 1024)
                )));
            }
            let data = std::fs::read(child.path()).map_err(FileError::Io)?;
            if is_zip(&data) {
                unzip(&path, &format!("{path}/"), data, 1, &mut budget, &mut out)?;
            } else if size > MAX_ENTRY_BYTES {
                return Err(FileError::TooLarge(format!(
                    "{path} is larger than {} MB",
                    MAX_ENTRY_BYTES / (1024 * 1024)
                )));
            } else {
                budget.take(size)?;
                out.push(Entry { path, bytes: data });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------
// Reading

/// The folder an entry is in: "recipes/soup" for "recipes/soup/soup.json", "" at the top.
fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The names a recipe's own photo goes by beside it, best first: Nextcloud's `full.jpg`, Mealie's
/// `images/original.webp`, Tandoor's `image.jpg`.
const PHOTO_NAMES: &[&str] = &["full", "original", "image"];

/// A recipe's photo from the export: in its folder or that folder's `images/`, under one of [`PHOTO_NAMES`].
fn photo_beside<'a>(entries: &'a [Entry], dir: &str) -> Option<&'a Entry> {
    let images = if dir.is_empty() {
        "images".to_owned()
    } else {
        format!("{dir}/images")
    };
    let in_dir = |e: &&Entry| {
        let parent = folder(&e.path);
        parent == dir || parent == images
    };
    let stem = |e: &Entry| {
        let file = e.path.rsplit('/').next().unwrap_or(&e.path);
        file.rsplit_once('.')
            .map_or(file, |(s, _)| s)
            .to_ascii_lowercase()
    };
    PHOTO_NAMES.iter().find_map(|name| {
        entries
            .iter()
            .filter(in_dir)
            .filter(|e| IMAGE_EXTENSIONS.contains(&extension(&e.path).as_str()))
            .find(|e| stem(e) == *name)
    })
}

/// A recipe document found in an export, with its format, before it is read.
struct Document<'a> {
    path: &'a str,
    format: Format,
    value: Value,
}

/// Reads every recipe in an export's entries. Entries that aren't recipes (other JSON, photos) are passed over;
/// one that looks like a recipe but can't be read is listed in `skipped`. Fails when no recipe is found, or too many.
pub fn read(entries: &[Entry]) -> Result<Contents, FileError> {
    let mut documents: Vec<Document> = Vec::new();
    let mut skipped = Vec::new();
    // Why the file isn't a recipe, when it is one file and isn't: said in the error.
    let mut not_recipe: Option<String> = None;
    for entry in entries {
        if IMAGE_EXTENSIONS.contains(&extension(&entry.path).as_str()) {
            continue;
        }
        let text = if is_gzip(&entry.bytes) {
            match gunzip(&entry.bytes) {
                Ok(text) => text,
                Err(why) => {
                    skipped.push(Skipped {
                        path: entry.path.clone(),
                        reason: why,
                    });
                    continue;
                }
            }
        } else {
            entry.bytes.clone()
        };
        let text = text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&text);
        let value: Value = match serde_json::from_slice(text) {
            Ok(value) => value,
            Err(e) => {
                let why = format!("not valid JSON ({e})");
                if entries.len() == 1 {
                    not_recipe = Some(why);
                } else if extension(&entry.path) != "zip" {
                    skipped.push(Skipped {
                        path: entry.path.clone(),
                        reason: why,
                    });
                }
                continue;
            }
        };
        let found = classify(&entry.path, value);
        if found.is_empty() && entries.len() == 1 {
            not_recipe = Some("it is JSON, but not a recipe in any of those formats".into());
        }
        documents.extend(found.into_iter().map(|(format, value)| Document {
            path: &entry.path,
            format,
            value,
        }));
    }

    // A photo beside a recipe is that recipe's only when the folder holds no other.
    let mut per_folder: HashMap<&str, usize> = HashMap::new();
    for d in &documents {
        *per_folder.entry(folder(d.path)).or_default() += 1;
    }

    let mut recipes = Vec::new();
    for doc in documents {
        let records = match read_document(&doc) {
            Ok(records) => records,
            Err(e) => {
                skipped.push(Skipped {
                    path: doc.path.to_owned(),
                    reason: e.to_string(),
                });
                continue;
            }
        };
        let own_folder = per_folder[folder(doc.path)] == 1 && records.len() == 1;
        for (record, inline) in records {
            let (photo, photo_problem) = match inline {
                Some(Ok(bytes)) => (Some((format!("{}#photo_data", doc.path), bytes)), None),
                Some(Err(why)) => (None, Some(why)),
                None if own_folder => (
                    photo_beside(entries, folder(doc.path))
                        .map(|e| (e.path.clone(), e.bytes.clone())),
                    None,
                ),
                None => (None, None),
            };
            recipes.push(Found {
                format: doc.format,
                path: doc.path.to_owned(),
                record,
                photo,
                photo_problem,
            });
        }
    }
    if recipes.is_empty() {
        return Err(FileError::NoRecipes(not_recipe.or_else(|| {
            skipped.first().map(|s| format!("{}: {}", s.path, s.reason))
        })));
    }
    if recipes.len() > MAX_RECIPES {
        return Err(FileError::TooManyRecipes(recipes.len()));
    }
    Ok(Contents { recipes, skipped })
}

/// A gzipped entry's contents, no larger than an entry may be.
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("can't be unpacked ({e})"))?;
    if out.len() as u64 > MAX_ENTRY_BYTES {
        return Err(format!(
            "unpacks to more than {} MB",
            MAX_ENTRY_BYTES / (1024 * 1024)
        ));
    }
    Ok(out)
}

/// Whether a JSON document is schema.org's: an object with `@context`, `@type` or `@graph`, or a list holding one.
fn is_schema_org(value: &Value) -> bool {
    let tagged = |v: &Value| {
        v.as_object()
            .is_some_and(|m| m.keys().any(|k| k.starts_with('@')))
    };
    match value {
        Value::Array(items) => items.iter().any(tagged),
        v => tagged(v),
    }
}

/// The recipe documents in one JSON file, with the format each is in. A schema.org document is one, however many
/// recipes it holds; a list of an app's recipes is one document for each.
fn classify(path: &str, value: Value) -> Vec<(Format, Value)> {
    if is_schema_org(&value) {
        let format = if nextcloud::is(path) {
            Format::NextcloudCookbook
        } else {
            Format::SchemaOrg
        };
        return vec![(format, value)];
    }
    let one = |value: Value| -> Option<(Format, Value)> {
        let map = value.as_object()?;
        let format = if paprika::is(map) {
            Format::Paprika
        } else if tandoor::is(map) {
            Format::Tandoor
        } else if mealie::is(map) {
            Format::Mealie
        } else {
            return None;
        };
        Some((format, value))
    };
    match value {
        Value::Array(items) => items.into_iter().filter_map(one).collect(),
        value => one(value).into_iter().collect(),
    }
}

/// The photo inside a recipe, when the format keeps one there: the bytes, or why they couldn't be read.
type InlinePhoto = Option<Result<Vec<u8>, String>>;

/// A recipe and the photo inside it.
type WithPhoto = (Record, InlinePhoto);

fn read_document(doc: &Document) -> Result<Vec<WithPhoto>, ReadError> {
    let app = || doc.value.as_object().ok_or(ReadError::Untitled);
    let records = match doc.format {
        Format::SchemaOrg => schema_org::read_value(&doc.value, None)?
            .into_iter()
            .map(|r| (r, None))
            .collect(),
        Format::NextcloudCookbook => schema_org::read_value(&doc.value, None)?
            .into_iter()
            .map(|r| (nextcloud::with_id(r), None))
            .collect(),
        Format::Mealie => vec![(mealie::read(app()?)?, None)],
        Format::Tandoor => vec![(tandoor::read(app()?)?, None)],
        Format::Paprika => {
            let (record, photo) = paprika::read(app()?)?;
            vec![(record, photo)]
        }
    };
    // A photo is taken only from the file; a link to one elsewhere stays in the recipe's images, not fetched.
    Ok(records
        .into_iter()
        .map(|(mut record, photo)| {
            record.hero_image = None;
            (record, photo)
        })
        .collect())
}

/// Reads a recipe an app's fields were turned into: `schema` is it as schema.org JSON-LD, `raw` the app's own
/// object, kept as the record's. `external_id` is the app's id for it, when it has one.
fn via_schema_org(
    schema: Value,
    raw: Value,
    external_id: Option<String>,
) -> Result<Record, ReadError> {
    let mut record = schema_org::read_value(&schema, None)?
        .into_iter()
        .next()
        .ok_or(ReadError::Untitled)?;
    if let Some(id) = external_id {
        record.external_id = id;
    } else if record.url.is_none() {
        // The hash of the app's own object, not of what it was turned into.
        record.external_id = format!(
            "sha256:{}",
            hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                raw.to_string().as_bytes()
            ))
        );
    }
    record.raw = raw;
    Ok(record)
}

/// The text of a field that may be missing, null or empty.
fn text<'a>(map: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| map.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A number field that may be missing, null, a string or zero.
fn number(map: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|k| match map.get(*k)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().replace(',', ".").parse().ok(),
            _ => None,
        })
        .filter(|n| n.is_finite() && *n > 0.0)
}

/// An amount as a cook writes it in a line: "2", "1.25", "0.333".
fn amount_text(amount: f64) -> String {
    let rounded = (amount * 1000.0).round() / 1000.0;
    let mut s = format!("{rounded:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

/// The names in a list of strings or of objects with a `name` ("Dinner", {"name": "Dinner"}).
fn names(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Value::String(s) => Some(s.trim().to_owned()),
            Value::Object(m) => m
                .get("name")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_owned()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests;
