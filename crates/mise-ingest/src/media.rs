//! Media: store the archive's photos by content hash, link them to variants, steps and foods, and make
//! web-size copies.
//!
//! Three passes, each resumable:
//! 1. **Store.** Every image a variant (`meta.media`) or ingredient line (`source_food.image`) references is
//!    hashed and placed under `MEDIA_ROOT` at [`original_path`], by hard link when the archive is on the same
//!    volume (no extra space) and by copy otherwise. `media_sources` remembers which archive path produced
//!    which media row, so a re-run only reads paths it has not stored yet. Missing files are counted, not fatal.
//! 2. **Link.** Hero images become `variant_media` role `hero`; step images role `step` with the step they
//!    belong to; each food gets `props.image_media_id` from the image its ingredient lines use most, unless a
//!    food already has one. Rebuilt from the references every run; existing links are kept.
//! 3. **Derivatives.** Each stored image gets a copy per [`WEB_SIZES`] (longest side, never upscaled) as JPEG,
//!    or PNG when it has real transparency. Runs on a thread pool; only media missing a size is processed.
//!
//! Step images are matched to steps through the source record: the bundle wrote one step row per raw step in
//! raw order, and `meta.media.steps[].index` is the raw step's `index` field. Indices usually run 1..n but
//! some records start at 2, so the step's position is looked up rather than computed from the index. A
//! record without raw steps falls back to reading the index as 1-based.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageReader};
use mise_core::media::{ImageKind, WEB_SIZES, derivative_path, fit_within, original_path};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Files read, hashed and placed per batch; each batch is committed on its own, so an interrupted run loses
/// at most one batch of work.
const STORE_BATCH: usize = 1000;
/// Images resized per batch in the derivatives pass.
const DERIVATIVE_BATCH: usize = 256;
/// Log progress after at least this many files.
const LOG_EVERY: u64 = 5000;
/// JPEG quality of web copies: visually clean on phones and the kitchen display at a fraction of the size.
const JPEG_QUALITY: u8 = 82;
/// Missing files logged individually before only counting them.
const MISSING_LOGGED: u64 = 20;

/// What to run and where.
pub struct MediaOptions {
    /// Archive root the referenced paths are relative to (the bundle's `media_root`). Needed to store.
    pub archive_root: Option<PathBuf>,
    /// Where mise keeps its photos (`MEDIA_ROOT`, "./data/media"): originals by hash, and their web copies.
    pub media_root: PathBuf,
    /// Process at most this many new files (store) and media (derivatives); for trial runs.
    pub limit: Option<usize>,
    /// Worker threads for hashing and resizing.
    pub jobs: usize,
}

/// What the store and link passes did, for the command's summary.
#[derive(Debug, Default)]
pub struct StoreStats {
    /// Distinct archive paths referenced by variants and ingredient lines.
    pub referenced: u64,
    /// Paths stored by an earlier run, skipped without reading.
    pub already_stored: u64,
    /// Files read this run, images or not.
    pub read: u64,
    /// Total size of the image files read.
    pub bytes_read: u64,
    /// Referenced files not in the archive; left to retry next run.
    pub missing: u64,
    /// Files that failed to read or to place under `media_root` for another reason; left to retry next run.
    pub unreadable: u64,
    /// Read files whose content is not an image (e.g. a saved error page); not stored.
    pub not_image: u64,
    /// New media rows (distinct content).
    pub new_media: u64,
    /// Read files whose content was already stored under another path.
    pub duplicates: u64,
    /// Images placed by hard link, taking no extra space (the archive is on the same volume).
    pub hard_linked: u64,
    /// Images placed by copy (the archive is on another volume).
    pub copied: u64,
    /// Content already on disk (stored earlier in this run under another path, or by an interrupted run).
    pub present: u64,
    /// Hero images newly linked to their version.
    pub hero_links: u64,
    /// Step images newly linked to their version, with or without their step.
    pub step_links: u64,
    /// Step images whose step could not be found; linked to the variant with no step.
    pub step_links_without_step: u64,
    /// Foods given a picture: the image their ingredient lines use most.
    pub food_images: u64,
}

/// What the web-copy pass did, for the command's summary.
#[derive(Debug, Default)]
pub struct DerivativeStats {
    /// Media needing at least one web size when the pass started (after `--limit`).
    pub queued: u64,
    /// Queued media given their missing web sizes this run.
    pub done: u64,
    /// Resized copies encoded and written.
    pub files_written: u64,
    /// Total size of the copies written.
    pub bytes_written: u64,
    /// Sizes the original already fit, hard-linked to the original instead of re-encoded.
    pub files_linked: u64,
    /// Originals that could not be decoded; marked so later runs skip them.
    pub decode_failures: u64,
    /// Originals that could not be read (left to retry next run).
    pub read_failures: u64,
    /// Media still without every web size when the pass finished.
    pub remaining: u64,
}

fn thread_pool(jobs: usize) -> Result<Arc<rayon::ThreadPool>> {
    Ok(Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.max(1))
            .build()?,
    ))
}

fn log_rate(done: u64, total: u64, started: Instant) -> (f64, u64) {
    let rate = done as f64 / started.elapsed().as_secs_f64().max(0.001);
    let eta = if rate > 0.0 {
        ((total.saturating_sub(done)) as f64 / rate) as u64
    } else {
        0
    };
    ((rate * 10.0).round() / 10.0, eta)
}

/// Order `root`-relative file paths by modification time, read from directory listings (no file is opened).
/// The archive was written in download order, so on a hard disk this turns scattered seeks into mostly
/// sequential reads (path order is not download order). Files that cannot be listed go last.
fn in_disk_order<T: Send>(
    root: &Path,
    items: Vec<T>,
    path_of: impl Fn(&T) -> &str + Sync,
    threads: &rayon::ThreadPool,
) -> Vec<T> {
    let mut dirs: Vec<&str> = items
        .iter()
        .map(|it| path_of(it).rsplit_once('/').map_or("", |(d, _)| d))
        .collect();
    dirs.sort_unstable();
    dirs.dedup();
    let modified: HashMap<String, std::time::SystemTime> = threads.install(|| {
        dirs.par_iter()
            .flat_map_iter(|dir| {
                let entries = std::fs::read_dir(root.join(dir))
                    .into_iter()
                    .flatten()
                    .flatten();
                entries.filter_map(move |e| {
                    let time = e.metadata().ok()?.modified().ok()?;
                    let name = e.file_name().into_string().ok()?;
                    Some((
                        if dir.is_empty() {
                            name
                        } else {
                            format!("{dir}/{name}")
                        },
                        time,
                    ))
                })
            })
            .collect()
    });
    let mut keyed: Vec<(Option<std::time::SystemTime>, T)> = items
        .into_iter()
        .map(|it| (modified.get(path_of(&it)).copied(), it))
        .collect();
    keyed.sort_by_key(|(time, _)| (time.is_none(), *time));
    keyed.into_iter().map(|(_, it)| it).collect()
}

// ---------------------------------------------------------------------------------------------------------
// Store

/// One archive file after reading: its content hash, format and where it was placed.
struct ReadFile {
    path: String,
    sha256: String,
    kind: ImageKind,
    bytes: i64,
    dims: Option<(u32, u32)>,
    placed: Placed,
}

enum Placed {
    HardLinked,
    Copied,
    Present,
}

enum ReadOutcome {
    Missing(String),
    Unreadable(String, String),
    /// Content is not an image; carries the first bytes for the log.
    NotImage(String, String),
    Read(ReadFile),
}

/// Store every referenced file not stored yet, then rebuild the links.
pub async fn store_and_link(pool: &PgPool, opts: &MediaOptions) -> Result<StoreStats> {
    let mut stats = StoreStats::default();
    let archive_root = opts
        .archive_root
        .as_deref()
        .context("--from (the archive root) is required to store media")?;
    anyhow::ensure!(
        archive_root.is_dir(),
        "archive root {} is not a directory",
        archive_root.display()
    );
    std::fs::create_dir_all(&opts.media_root)
        .with_context(|| format!("creating media root {}", opts.media_root.display()))?;

    // Ordered by path so a --limit trial run takes whole recipes; reading order is decided below.
    let referenced: Vec<String> = sqlx::query_scalar(REFERENCED_PATHS).fetch_all(pool).await?;
    let stored: HashSet<String> = sqlx::query_scalar("SELECT path FROM media_sources")
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    stats.referenced = referenced.len() as u64;
    let mut todo: Vec<String> = referenced
        .into_iter()
        .filter(|p| !stored.contains(p))
        .collect();
    stats.already_stored = stats.referenced - todo.len() as u64;
    if let Some(limit) = opts.limit {
        todo.truncate(limit);
    }
    tracing::info!(
        referenced = stats.referenced,
        already_stored = stats.already_stored,
        to_read = todo.len(),
        "storing originals"
    );

    let pool_threads = thread_pool(opts.jobs)?;
    let todo = in_disk_order(archive_root, todo, |p| p.as_str(), &pool_threads);
    tracing::info!("ordered by modification time");
    let started = Instant::now();
    let total = todo.len() as u64;
    let mut processed = 0u64;
    let mut next_log = LOG_EVERY;
    for chunk in todo.chunks(STORE_BATCH) {
        let chunk = chunk.to_vec();
        let threads = pool_threads.clone();
        let archive = archive_root.to_path_buf();
        let media_root = opts.media_root.clone();
        let outcomes: Vec<ReadOutcome> = tokio::task::spawn_blocking(move || {
            threads.install(|| {
                chunk
                    .par_iter()
                    .map(|p| read_and_place(&archive, &media_root, p))
                    .collect()
            })
        })
        .await?;
        processed += outcomes.len() as u64;
        write_store_batch(pool, outcomes, &mut stats).await?;
        if processed >= next_log || processed == total {
            next_log = processed + LOG_EVERY;
            let (per_s, eta_s) = log_rate(processed, total, started);
            tracing::info!(
                processed,
                total,
                new_media = stats.new_media,
                missing = stats.missing,
                per_s,
                eta_s,
                "storing"
            );
        }
    }

    link(pool, &mut stats).await?;
    Ok(stats)
}

/// Every distinct archive path referenced by a variant's hero or step images or by an ingredient line.
const REFERENCED_PATHS: &str = "
    SELECT meta->'media'->>'hero' AS path FROM variants WHERE meta->'media'->>'hero' IS NOT NULL
    UNION
    SELECT i #>> '{}' FROM variants v, jsonb_array_elements(v.meta->'media'->'steps') s, jsonb_array_elements(s->'images') i
    WHERE jsonb_typeof(v.meta->'media'->'steps') = 'array' AND jsonb_typeof(s->'images') = 'array'
    UNION
    SELECT source_food->>'image' FROM ingredient_lines WHERE source_food->>'image' IS NOT NULL
    ORDER BY 1";

fn read_and_place(archive_root: &Path, media_root: &Path, path: &str) -> ReadOutcome {
    let source = archive_root.join(path);
    let data = match std::fs::read(&source) {
        Ok(d) => d,
        Err(e) if e.kind() == ErrorKind::NotFound => return ReadOutcome::Missing(path.to_string()),
        Err(e) => return ReadOutcome::Unreadable(path.to_string(), e.to_string()),
    };
    // Judged by content, not extension: the archive holds saved S3 error pages named like images.
    let Some(kind) = ImageKind::sniff(&data) else {
        return ReadOutcome::NotImage(
            path.to_string(),
            String::from_utf8_lossy(&data[..data.len().min(60)]).into_owned(),
        );
    };
    let sha256 = hex::encode(Sha256::digest(&data));
    let dims = image_dimensions(&data, kind);

    let dest = media_root.join(original_path(&sha256, kind.ext()));
    let placed = match place(&source, &dest) {
        Ok(p) => p,
        Err(e) => {
            return ReadOutcome::Unreadable(
                path.to_string(),
                format!("placing {}: {e}", dest.display()),
            );
        }
    };
    ReadOutcome::Read(ReadFile {
        path: path.to_string(),
        sha256,
        kind,
        bytes: data.len() as i64,
        dims,
        placed,
    })
}

/// Width and height from the image header, without decoding pixels.
fn image_dimensions(data: &[u8], kind: ImageKind) -> Option<(u32, u32)> {
    let format = match kind {
        ImageKind::Jpeg => image::ImageFormat::Jpeg,
        ImageKind::Png => image::ImageFormat::Png,
        ImageKind::Webp => image::ImageFormat::WebP,
        ImageKind::Gif => image::ImageFormat::Gif,
    };
    ImageReader::with_format(Cursor::new(data), format)
        .into_dimensions()
        .ok()
}

/// Put `source` at `dest`: hard link when possible (same volume), else copy. Content-addressed, so an
/// existing `dest` already holds these bytes.
fn place(source: &Path, dest: &Path) -> std::io::Result<Placed> {
    if dest.exists() {
        return Ok(Placed::Present);
    }
    std::fs::create_dir_all(dest.parent().expect("original path has a parent"))?;
    match std::fs::hard_link(source, dest) {
        Ok(()) => Ok(Placed::HardLinked),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(Placed::Present),
        Err(_) => {
            // Different volume or a filesystem without hard links. Copy via a temporary name so a crash never
            // leaves a partial file at the content-addressed path.
            let tmp = dest.with_extension("partial");
            std::fs::copy(source, &tmp)?;
            match std::fs::rename(&tmp, dest) {
                Ok(()) => Ok(Placed::Copied),
                Err(_) if dest.exists() => {
                    let _ = std::fs::remove_file(&tmp);
                    Ok(Placed::Present)
                }
                Err(e) => Err(e),
            }
        }
    }
}

async fn write_store_batch(
    pool: &PgPool,
    outcomes: Vec<ReadOutcome>,
    stats: &mut StoreStats,
) -> Result<()> {
    let mut files = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        match outcome {
            ReadOutcome::Missing(path) => {
                stats.missing += 1;
                if stats.missing <= MISSING_LOGGED {
                    tracing::warn!(path, "referenced file is missing");
                }
            }
            ReadOutcome::Unreadable(path, error) => {
                stats.unreadable += 1;
                tracing::warn!(path, error, "could not store file");
            }
            ReadOutcome::NotImage(path, head) => {
                stats.read += 1;
                stats.not_image += 1;
                if stats.not_image <= MISSING_LOGGED {
                    tracing::warn!(path, head, "referenced file is not an image");
                }
            }
            ReadOutcome::Read(f) => {
                stats.read += 1;
                stats.bytes_read += f.bytes as u64;
                match f.placed {
                    Placed::HardLinked => stats.hard_linked += 1,
                    Placed::Copied => stats.copied += 1,
                    Placed::Present => stats.present += 1,
                }
                files.push(f);
            }
        }
    }
    if files.is_empty() {
        return Ok(());
    }

    // One media row per distinct content; files whose content is already stored only gain a source path.
    let mut seen = HashSet::new();
    let new: Vec<&ReadFile> = files
        .iter()
        .filter(|f| seen.insert(f.sha256.as_str()))
        .collect();
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO media (id, sha256, mime, bytes, width, height, path)
         SELECT * FROM UNNEST($1::uuid[], $2::text[], $3::text[], $4::bigint[], $5::int[], $6::int[], $7::text[])
         ON CONFLICT (sha256) DO NOTHING",
    )
    .bind(new.iter().map(|_| Uuid::now_v7()).collect::<Vec<_>>())
    .bind(new.iter().map(|f| f.sha256.clone()).collect::<Vec<_>>())
    .bind(new.iter().map(|f| f.kind.mime().to_string()).collect::<Vec<_>>())
    .bind(new.iter().map(|f| f.bytes).collect::<Vec<_>>())
    .bind(new.iter().map(|f| f.dims.map(|d| d.0 as i32)).collect::<Vec<_>>())
    .bind(new.iter().map(|f| f.dims.map(|d| d.1 as i32)).collect::<Vec<_>>())
    .bind(new.iter().map(|f| original_path(&f.sha256, f.kind.ext())).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await
    .context("inserting media")?
    .rows_affected();
    stats.new_media += inserted;
    stats.duplicates += files.len() as u64 - inserted;

    sqlx::query(
        "INSERT INTO media_sources (path, media_id)
         SELECT t.path, m.id FROM UNNEST($1::text[], $2::text[]) AS t(path, sha) JOIN media m ON m.sha256 = t.sha
         ON CONFLICT (path) DO NOTHING",
    )
    .bind(files.iter().map(|f| f.path.clone()).collect::<Vec<_>>())
    .bind(files.iter().map(|f| f.sha256.clone()).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await
    .context("inserting media sources")?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Link

/// Link stored media to variants, steps and foods. Only adds: links that exist are left alone.
async fn link(pool: &PgPool, stats: &mut StoreStats) -> Result<()> {
    let mut tx = pool.begin().await?;
    stats.hero_links = sqlx::query(
        "INSERT INTO variant_media (variant_id, media_id, role, position)
         SELECT v.id, s.media_id, 'hero', 0 FROM variants v JOIN media_sources s ON s.path = v.meta->'media'->>'hero'
         ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await
    .context("linking hero images")?
    .rows_affected();

    // Each meta step image, numbered across the variant; its step found through the raw step with that index.
    let step_links: Vec<(i64, bool)> = sqlx::query_as(
        "WITH img AS (
             SELECT v.id AS variant_id, v.source_record_id, (s->>'index')::int AS idx, i #>> '{}' AS path,
                    (row_number() OVER (PARTITION BY v.id ORDER BY so, io) - 1)::int AS position
             FROM variants v,
                  jsonb_array_elements(v.meta->'media'->'steps') WITH ORDINALITY a(s, so),
                  jsonb_array_elements(s->'images') WITH ORDINALITY b(i, io)
             WHERE jsonb_typeof(v.meta->'media'->'steps') = 'array' AND jsonb_typeof(s->'images') = 'array'
         ), raw_step AS (
             SELECT sr.id AS record_id, (e->>'index')::int AS idx, (min(ord) - 1)::int AS position
             FROM source_records sr, jsonb_array_elements(sr.raw->'steps') WITH ORDINALITY x(e, ord)
             WHERE jsonb_typeof(sr.raw->'steps') = 'array'
             GROUP BY 1, 2
         ), placed AS (
             SELECT img.variant_id, ms.media_id, st.id AS step_id, img.position
             FROM img
             JOIN media_sources ms ON ms.path = img.path
             LEFT JOIN raw_step r ON r.record_id = img.source_record_id AND r.idx = img.idx
             -- Without raw steps to look in, read the index as 1-based.
             LEFT JOIN steps st ON st.variant_id = img.variant_id AND st.position = CASE
                 WHEN EXISTS (SELECT 1 FROM raw_step x WHERE x.record_id = img.source_record_id) THEN r.position
                 ELSE img.idx - 1 END
         ), ins AS (
             INSERT INTO variant_media (variant_id, media_id, role, step_id, position)
             SELECT variant_id, media_id, 'step', step_id, position FROM placed
             ON CONFLICT DO NOTHING
             RETURNING step_id
         )
         SELECT count(*), step_id IS NOT NULL FROM ins GROUP BY 2",
    )
    .fetch_all(&mut *tx)
    .await
    .context("linking step images")?;
    for (n, with_step) in step_links {
        stats.step_links += n as u64;
        if !with_step {
            stats.step_links_without_step += n as u64;
        }
    }

    // A food's picture: the stored image its ingredient lines use most. A food that has one keeps it.
    stats.food_images = sqlx::query(
        "WITH pick AS (
             SELECT DISTINCT ON (il.food_id) il.food_id, ms.media_id
             FROM ingredient_lines il JOIN media_sources ms ON ms.path = il.source_food->>'image'
             WHERE il.food_id IS NOT NULL
             GROUP BY il.food_id, ms.media_id
             ORDER BY il.food_id, count(*) DESC, ms.media_id
         )
         UPDATE foods f SET props = f.props || jsonb_build_object('image_media_id', pick.media_id)
         FROM pick WHERE f.id = pick.food_id AND NOT f.props ? 'image_media_id'",
    )
    .execute(&mut *tx)
    .await
    .context("linking food images")?
    .rows_affected();
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------
// Derivatives

struct Derivative {
    max_side: i32,
    path: String,
    mime: &'static str,
    bytes: i64,
    width: i32,
    height: i32,
    /// A hard link to the original rather than a new encoding.
    linked: bool,
}

enum DerivativeOutcome {
    Made(Uuid, Vec<Derivative>),
    DecodeFailed(Uuid, String),
    ReadFailed(Uuid, String),
}

/// Make the missing web sizes of every decodable stored image.
pub async fn make_derivatives(pool: &PgPool, opts: &MediaOptions) -> Result<DerivativeStats> {
    let mut stats = DerivativeStats::default();
    let sizes: Vec<i32> = WEB_SIZES.iter().map(|&s| s as i32).collect();
    let mut todo: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT m.id, m.sha256, m.path FROM media m
         WHERE m.decode_error IS NULL AND m.mime LIKE 'image/%'
           AND (SELECT count(*) FROM media_derivatives d WHERE d.media_id = m.id AND d.max_side = ANY($1)) < cardinality($1)
         ORDER BY m.path",
    )
    .bind(&sizes)
    .fetch_all(pool)
    .await?;
    let waiting = todo.len() as u64;
    if let Some(limit) = opts.limit {
        todo.truncate(limit);
    }
    stats.queued = todo.len() as u64;
    tracing::info!(waiting, queued = stats.queued, sizes = ?WEB_SIZES, jobs = opts.jobs, "making web copies");

    let threads = thread_pool(opts.jobs)?;
    // Originals are hard links to the archive files, so they share their modification times and disk layout.
    let todo = in_disk_order(&opts.media_root, todo, |m| m.2.as_str(), &threads);
    let started = Instant::now();
    let mut next_log = LOG_EVERY;
    for chunk in todo.chunks(DERIVATIVE_BATCH) {
        let chunk = chunk.to_vec();
        let threads = threads.clone();
        let media_root = opts.media_root.clone();
        let outcomes: Vec<DerivativeOutcome> = tokio::task::spawn_blocking(move || {
            threads.install(|| {
                chunk
                    .par_iter()
                    .map(|(id, sha, path)| derive(&media_root, *id, sha, path))
                    .collect()
            })
        })
        .await?;
        write_derivatives(pool, outcomes, &mut stats).await?;
        if stats.done >= next_log || stats.done == stats.queued {
            next_log = stats.done + LOG_EVERY;
            let (per_s, eta_s) = log_rate(stats.done, stats.queued, started);
            let written_mb = stats.bytes_written / 1_000_000;
            tracing::info!(
                done = stats.done,
                queued = stats.queued,
                failures = stats.decode_failures + stats.read_failures,
                written_mb,
                per_s,
                eta_s,
                "making web copies"
            );
        }
    }
    stats.remaining = waiting - stats.done - stats.decode_failures;
    Ok(stats)
}

fn derive(media_root: &Path, id: Uuid, sha256: &str, path: &str) -> DerivativeOutcome {
    let data = match std::fs::read(media_root.join(path)) {
        Ok(d) => d,
        Err(e) => return DerivativeOutcome::ReadFailed(id, e.to_string()),
    };
    let Some(kind) = ImageKind::sniff(&data) else {
        return DerivativeOutcome::DecodeFailed(id, "not an image".into());
    };
    let (image, upright) = match decode(&data) {
        Ok(decoded) => decoded,
        Err(e) => return DerivativeOutcome::DecodeFailed(id, e.to_string()),
    };
    // An original that is already small enough, upright and in a format every browser shows is used as is.
    let reusable = (upright && kind != ImageKind::Gif)
        .then(|| (media_root.join(path), kind, data.len() as i64));
    drop(data);
    match write_web_sizes(media_root, sha256, image, reusable) {
        Ok(made) => DerivativeOutcome::Made(id, made),
        Err(e) => DerivativeOutcome::ReadFailed(id, format!("writing web copies: {e}")),
    }
}

/// Decode and turn upright per the EXIF orientation, so web copies display the right way up everywhere.
/// Also says whether the stored bytes were already upright.
fn decode(data: &[u8]) -> image::ImageResult<(DynamicImage, bool)> {
    let mut decoder = ImageReader::new(Cursor::new(data))
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok((
        image,
        orientation == image::metadata::Orientation::NoTransforms,
    ))
}

/// Resize to each web size (largest first, each from the previous to save work) and encode. A size the
/// original already fits is a hard link to the original when `reusable` allows it, instead of a re-encode.
fn write_web_sizes(
    media_root: &Path,
    sha256: &str,
    image: DynamicImage,
    reusable: Option<(PathBuf, ImageKind, i64)>,
) -> Result<Vec<Derivative>> {
    let (orig_w, orig_h) = (image.width(), image.height());
    // PNG only when transparency is actually used (ingredient cut-outs); everything else is a JPEG photo.
    let transparent = image.color().has_alpha() && image.to_rgba8().pixels().any(|p| p.0[3] < 255);
    let mut current = if transparent {
        DynamicImage::ImageRgba8(image.into_rgba8())
    } else {
        DynamicImage::ImageRgb8(image.into_rgb8())
    };
    let mut sizes = WEB_SIZES.to_vec();
    sizes.sort_unstable_by(|a, b| b.cmp(a));

    let mut made = Vec::with_capacity(sizes.len());
    for max_side in sizes {
        let (w, h) = fit_within(orig_w, orig_h, max_side);
        if let (Some((original, kind, bytes)), true) = (&reusable, (w, h) == (orig_w, orig_h)) {
            let rel = derivative_path(sha256, max_side, kind.ext());
            let dest = media_root.join(&rel);
            // Replace whatever an earlier run left, so the link always points at the original.
            match std::fs::remove_file(&dest) {
                Err(e) if e.kind() != ErrorKind::NotFound => {
                    return Err(e).with_context(|| format!("replacing {rel}"));
                }
                _ => {}
            }
            place(original, &dest).with_context(|| format!("linking {rel}"))?;
            made.push(Derivative {
                max_side: max_side as i32,
                path: rel,
                mime: kind.mime(),
                bytes: *bytes,
                width: w as i32,
                height: h as i32,
                linked: true,
            });
            continue;
        }
        if (w, h) != (current.width(), current.height()) {
            current = current.resize_exact(w, h, FilterType::CatmullRom);
        }
        let mut out = Vec::new();
        let (ext, mime) = if transparent {
            PngEncoder::new_with_quality(&mut out, CompressionType::Best, PngFilter::Adaptive)
                .write_image(current.as_bytes(), w, h, current.color().into())?;
            ("png", "image/png")
        } else {
            JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY).write_image(
                current.as_bytes(),
                w,
                h,
                current.color().into(),
            )?;
            ("jpg", "image/jpeg")
        };
        let rel = derivative_path(sha256, max_side, ext);
        let dest = media_root.join(&rel);
        std::fs::create_dir_all(dest.parent().expect("derivative path has a parent"))?;
        // Written beside and renamed over: writing in place through a path an earlier run hard-linked to the
        // original would overwrite the original (and the archive file it is linked to).
        let tmp = dest.with_extension("partial");
        std::fs::write(&tmp, &out).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("renaming to {}", dest.display()))?;
        made.push(Derivative {
            max_side: max_side as i32,
            path: rel,
            mime,
            bytes: out.len() as i64,
            width: w as i32,
            height: h as i32,
            linked: false,
        });
    }
    Ok(made)
}

async fn write_derivatives(
    pool: &PgPool,
    outcomes: Vec<DerivativeOutcome>,
    stats: &mut DerivativeStats,
) -> Result<()> {
    let mut rows: Vec<(Uuid, Derivative)> = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();
    for outcome in outcomes {
        match outcome {
            DerivativeOutcome::Made(id, made) => {
                stats.done += 1;
                for d in &made {
                    if d.linked {
                        stats.files_linked += 1;
                    } else {
                        stats.files_written += 1;
                        stats.bytes_written += d.bytes as u64;
                    }
                }
                rows.extend(made.into_iter().map(|d| (id, d)));
            }
            DerivativeOutcome::DecodeFailed(id, error) => {
                stats.decode_failures += 1;
                tracing::warn!(%id, error, "could not decode image");
                failed.push((id, error));
            }
            DerivativeOutcome::ReadFailed(id, error) => {
                stats.read_failures += 1;
                tracing::warn!(%id, error, "could not make web copies");
            }
        }
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO media_derivatives (media_id, max_side, path, mime, bytes, width, height)
         SELECT * FROM UNNEST($1::uuid[], $2::int[], $3::text[], $4::text[], $5::bigint[], $6::int[], $7::int[])
         ON CONFLICT (media_id, max_side) DO UPDATE
         SET path = excluded.path, mime = excluded.mime, bytes = excluded.bytes, width = excluded.width, height = excluded.height",
    )
    .bind(rows.iter().map(|r| r.0).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.1.max_side).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.1.path.clone()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.1.mime.to_string()).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.1.bytes).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.1.width).collect::<Vec<_>>())
    .bind(rows.iter().map(|r| r.1.height).collect::<Vec<_>>())
    .execute(&mut *tx)
    .await
    .context("inserting web copies")?;
    sqlx::query("UPDATE media m SET decode_error = t.e FROM UNNEST($1::uuid[], $2::text[]) AS t(id, e) WHERE m.id = t.id")
        .bind(failed.iter().map(|f| f.0).collect::<Vec<_>>())
        .bind(failed.iter().map(|f| f.1.clone()).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn orders_by_modification_time_with_unlisted_last() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/images")).unwrap();
        std::fs::create_dir_all(root.join("b/images")).unwrap();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        for (path, offset) in [
            ("a/images/hero.jpg", 30),
            ("b/images/hero.jpg", 10),
            ("b/images/step-01-1.jpg", 20),
        ] {
            let file = std::fs::File::create(root.join(path)).unwrap();
            file.set_modified(base + Duration::from_secs(offset))
                .unwrap();
        }
        let threads = thread_pool(2).unwrap();
        let paths: Vec<String> = [
            "a/images/hero.jpg",
            "missing/x.jpg",
            "b/images/step-01-1.jpg",
            "b/images/hero.jpg",
        ]
        .map(String::from)
        .into();
        let ordered = in_disk_order(root, paths, |p| p.as_str(), &threads);
        assert_eq!(
            ordered,
            [
                "b/images/hero.jpg",
                "b/images/step-01-1.jpg",
                "a/images/hero.jpg",
                "missing/x.jpg"
            ]
        );
    }
}
