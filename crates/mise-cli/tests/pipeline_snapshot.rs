//! The import pipeline end to end: the demo recipes (`fixtures/demo-bundle`, written for mise) go through every
//! stage the `mise` command runs, in its order (import, match foods, apply curation, group dishes, derive options,
//! parse steps), and what comes out is compared with the snapshots committed in `fixtures/demo-bundle/snapshots/`:
//!
//! - `pipeline.txt`: what each stage reported, the row count of each table it fills, the open review items and the
//!   food tree;
//! - `<dish slug>.txt`: each dish page (its options and versions), and every version at 1 to 6 servings as the API
//!   renders it (`mise_api::variants::get`), ingredients and steps.
//!
//! So a change to grouping, matching, step parsing or rounding shows up as a diff of plain text. The snapshots hold
//! nothing that varies from run to run: no ids or timestamps, and everything in a fixed order.
//!
//! When the output changes on purpose, rewrite the snapshots and review them with `git diff`:
//! `MISE_UPDATE_SNAPSHOTS=1 cargo test -p mise-cli --test pipeline_snapshot`.
//!
//! Like the API tests, it makes a throwaway database on the server DATABASE_URL names and drops it afterwards; with
//! no database it skips on a laptop and fails in CI.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{Extension, Json, extract::State};
use mise_api::{
    AppState,
    auth::{Auth, AuthConfig, CurrentUser, Via},
    dishes,
    extract::{Path as ApiPath, Query},
    variants::{self, RenderParams},
};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};

/// Set to rewrite the snapshots instead of comparing with them.
const UPDATE: &str = "MISE_UPDATE_SNAPSHOTS";

/// The serving counts every version is rendered at.
const SERVINGS: std::ops::RangeInclusive<u32> = 1..=6;

/// The tables the pipeline fills, counted in `pipeline.txt`.
const TABLES: [&str; 15] = [
    "sources",
    "source_records",
    "variants",
    "variant_yields",
    "ingredient_lines",
    "line_amounts",
    "steps",
    "foods",
    "food_aliases",
    "food_relations",
    "food_measures",
    "dishes",
    "dish_options",
    "variant_attributes",
    "review_items",
];

fn bundle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/demo-bundle")
}

fn snapshot_dir() -> PathBuf {
    bundle_dir().join("snapshots")
}

#[tokio::test]
async fn demo_bundle_through_every_stage_matches_the_snapshots() {
    let Some(db) = Database::create().await else {
        eprintln!("DATABASE_URL is unset or unreachable; skipping");
        return;
    };
    // Declared after the database, so it closes first.
    let pool = mise_db::connect(&db.url()).await.unwrap();
    mise_db::migrate(&pool).await.unwrap();

    let mut snapshots = BTreeMap::new();
    let mut summary = run_pipeline(&pool).await;
    check_counts(&pool).await;
    summary.push_str(&tables(&pool).await);
    summary.push_str(&review_items(&pool).await);
    summary.push_str(&food_tree(&pool).await);
    snapshots.insert("pipeline.txt".to_string(), summary);

    let state = AppState {
        pool: pool.clone(),
        media_root: PathBuf::from("./no-media"),
        auth: Arc::new(Auth::new(AuthConfig {
            local: true,
            oidc: None,
            public_base_url: None,
            session_days: 30,
        })),
    };
    let slugs: Vec<String> = sqlx::query_scalar("SELECT slug FROM dishes ORDER BY slug")
        .fetch_all(&pool)
        .await
        .unwrap();
    for slug in slugs {
        let page = dish_page(&state, &slug).await;
        snapshots.insert(format!("{slug}.txt"), page);
    }

    compare(&snapshots);
}

/// Every stage, in the order `docs/development.md` gives, with what each one reported.
async fn run_pipeline(pool: &PgPool) -> String {
    let dir = bundle_dir();
    let mut out = String::from("# The demo bundle through every stage\n");
    let import = mise_ingest::bundle::import_bundle(pool, &dir)
        .await
        .expect("import the demo bundle");
    stage(&mut out, "Import", &import);
    let matched = mise_ingest::foods::match_foods(pool)
        .await
        .expect("match foods");
    stage(&mut out, "Match foods", &matched);
    let curated = mise_ingest::curate::apply_file(pool, &dir.join("curation.json"))
        .await
        .expect("apply the demo curation");
    writeln!(out, "\n## Apply curation\n\n{} changes", curated.len()).unwrap();
    let grouped = mise_ingest::dishes::group_dishes(pool)
        .await
        .expect("group dishes");
    stage(&mut out, "Group dishes", &grouped);
    let options = mise_ingest::options::derive_options(pool)
        .await
        .expect("derive options");
    stage(&mut out, "Derive options", &options);
    let steps = mise_ingest::steps::parse_steps(pool)
        .await
        .expect("parse steps");
    stage(&mut out, "Parse steps", &steps);
    out
}

/// A stage's report, one field a line.
fn stage(out: &mut String, name: &str, stats: &impl std::fmt::Debug) {
    writeln!(out, "\n## {name}\n\n{stats:#?}").unwrap();
}

/// What the demo bundle's README promises, checked outright rather than only through the snapshot.
async fn check_counts(pool: &PgPool) {
    let count = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(pool)
            .await
            .unwrap()
    };
    assert_eq!(count("SELECT count(*) FROM variants").await, 5, "versions");
    assert_eq!(count("SELECT count(*) FROM dishes").await, 4, "dishes");
    assert_eq!(
        count("SELECT count(*) FROM variants WHERE dish_id IS NULL").await,
        0,
        "every version belongs to a dish"
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM dishes d JOIN variants v ON v.dish_id = d.id
             WHERE d.title = 'Black Bean Tacos'"
        )
        .await,
        2,
        "both taco versions are one dish"
    );
    assert_eq!(
        count("SELECT count(*) FROM ingredient_lines WHERE food_id IS NULL").await,
        0,
        "every ingredient line is matched to a food"
    );
    assert_eq!(
        count("SELECT count(*) FROM steps WHERE template IS NULL").await,
        0,
        "every step is parsed"
    );
}

/// The row count of each table the pipeline fills.
async fn tables(pool: &PgPool) -> String {
    let mut out = String::from("\n## Rows\n\n");
    for table in TABLES {
        // The table names are the constants above, not input.
        let n: i64 = sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(pool)
            .await
            .unwrap();
        writeln!(out, "{table}: {n}").unwrap();
    }
    out
}

/// What the pipeline left for a person to decide: each open item's kind and subject.
async fn review_items(pool: &PgPool) -> String {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT kind, subject::text FROM review_items WHERE status = 'open' ORDER BY kind, subject::text",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = String::from("\n## Open review items\n\n");
    if rows.is_empty() {
        out.push_str("none\n");
    }
    for (kind, subject) in rows {
        writeln!(out, "- {kind}: {}", redact_ids(&subject)).unwrap();
    }
    out
}

/// Every food by name, with its parent, properties and measures, which matching and curation set.
async fn food_tree(pool: &PgPool) -> String {
    let rows: Vec<(String, Option<String>, serde_json::Value, Option<String>)> = sqlx::query_as(
        "SELECT f.name, p.name, f.props,
                (SELECT string_agg(CASE m.kind WHEN 'density' THEN format('%s ml weighs %s g', round(m.ml, 1), round(m.grams, 1))
                                    ELSE concat_ws(', ', format('one %s', m.item),
                                                   CASE WHEN m.ml IS NOT NULL THEN format('%s ml', round(m.ml, 1)) END,
                                                   CASE WHEN m.grams IS NOT NULL THEN format('%s g', round(m.grams, 1)) END) END,
                                   '; ' ORDER BY m.kind, m.item)
                 FROM food_measures m WHERE m.food_id = f.id)
         FROM foods f LEFT JOIN foods p ON p.id = f.parent_id ORDER BY f.name",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = String::from("\n## Foods\n\n");
    for (name, parent, props, measures) in rows {
        out.push_str("- ");
        out.push_str(&name);
        if let Some(parent) = parent {
            write!(out, " (kind of {parent})").unwrap();
        }
        // jsonb comes back with its keys sorted, so this is stable.
        if props.as_object().is_some_and(|p| !p.is_empty()) {
            write!(out, " {}", redact_ids(&props.to_string())).unwrap();
        }
        if let Some(measures) = measures {
            write!(out, " [{measures}]").unwrap();
        }
        out.push('\n');
    }
    out
}

/// Any UUID in `s` replaced by `<id>`: ids change from run to run.
fn redact_ids(s: &str) -> String {
    let is_uuid = |w: &str| w.len() == 36 && uuid::Uuid::parse_str(w).is_ok();
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if rest.len() >= 36 && rest.is_char_boundary(36) && is_uuid(&rest[..36]) {
            out.push_str("<id>");
            rest = &rest[36..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Who reads the pages: an account in a household of its own. The demo recipes belong to no household, so every
/// account sees all of them, as this one does.
fn reader() -> Extension<CurrentUser> {
    Extension(CurrentUser {
        id: uuid::Uuid::nil(),
        username: "reader".into(),
        is_admin: false,
        household_id: uuid::Uuid::nil(),
        via: Via::Session,
    })
}

/// A dish page: the dish with its options and versions, then each version at every serving count in `SERVINGS`,
/// as the API serves them. Versions are numbered in the order the page lists them.
async fn dish_page(state: &AppState, slug: &str) -> String {
    // Options and their choices keep the order the API gives: `derive_options` settles it from the data alone.
    let Json(dish) = dishes::get(State(state.clone()), reader(), ApiPath(slug.to_string()))
        .await
        .unwrap_or_else(|e| panic!("dish {slug}: {e}"));
    let mut versions: Vec<&dishes::VariantSummary> = dish.variants.iter().collect();
    // The API lists the newest first; title and subtitle settle a tie, so the order never varies between runs.
    versions.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.subtitle.cmp(&b.subtitle))
    });
    let number = |id: uuid::Uuid| {
        versions
            .iter()
            .position(|v| v.id == id)
            .map_or("none".to_string(), |i| (i + 1).to_string())
    };

    let mut out = String::new();
    writeln!(out, "# {}", dish.title).unwrap();
    if let Some(s) = &dish.subtitle {
        writeln!(out, "{s}").unwrap();
    }
    writeln!(
        out,
        "\nslug {} · {} · opens on version {}",
        dish.slug,
        dish.content_type,
        dish.default_variant.map_or("none".to_string(), number)
    )
    .unwrap();

    out.push_str("\n## Options\n\n");
    if dish.options.is_empty() {
        out.push_str("none\n");
    }
    for o in &dish.options {
        writeln!(
            out,
            "- {} ({}), default {}",
            o.label,
            o.key,
            o.default.as_deref().unwrap_or("none")
        )
        .unwrap();
        for c in &o.choices {
            writeln!(
                out,
                "  - {}: {} · {} · {}",
                c.value,
                c.label,
                match c.variants {
                    1 => "1 version".to_string(),
                    n => format!("{n} versions"),
                },
                c.years.as_deref().unwrap_or("no years")
            )
            .unwrap();
        }
    }

    out.push_str("\n## Versions, newest first\n\n");
    for (i, v) in versions.iter().enumerate() {
        writeln!(
            out,
            "{}. {}{} · published {} · yields {} · {} · attributes {}{}",
            i + 1,
            v.title,
            v.subtitle
                .as_deref()
                .map_or(String::new(), |s| format!(", {s}")),
            v.published_at
                .map_or("unknown".to_string(), |d| d.date_naive().to_string()),
            list(&v.yields),
            if v.has_steps { "steps" } else { "no steps" },
            v.attributes,
            if v.image.is_some() { " · photo" } else { "" },
        )
        .unwrap();
    }

    for (i, v) in versions.iter().enumerate() {
        version(&mut out, state, i + 1, v.id).await;
    }
    out
}

/// One version: what it says about itself, then its ingredients and steps at each serving count.
async fn version(out: &mut String, state: &AppState, number: usize, id: uuid::Uuid) {
    let mut first = true;
    for n in SERVINGS {
        let Json(v) = variants::get(
            State(state.clone()),
            reader(),
            ApiPath(id),
            Query(RenderParams {
                servings: Some(f64::from(n)),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("version {number} at {n} servings: {e}"));
        if first {
            first = false;
            writeln!(out, "\n## Version {number}: {}", v.title).unwrap();
            if let Some(s) = &v.subtitle {
                writeln!(out, "{s}").unwrap();
            }
            writeln!(out, "\nyields {}", list(&v.yields)).unwrap();
            if let Some(d) = &v.description {
                writeln!(out, "{d}").unwrap();
            }
            // serde_json keeps object keys sorted, so this is stable.
            writeln!(out, "meta {}", v.meta).unwrap();
        }

        let how = match (v.published_yield, v.official) {
            (true, true) => "as published",
            (true, false) => "a published yield, partly derived",
            (false, _) => "derived",
        };
        let noun = if n == 1 { "serving" } else { "servings" };
        writeln!(out, "\n### Version {number} at {n} {noun} ({how})\n").unwrap();
        for i in &v.ingredients {
            let mut notes = vec![];
            // The food the line was matched to, when it isn't simply the line's own name.
            match &i.food {
                Some(food) if *food == i.name => {}
                Some(food) => notes.push(format!("food: {food}")),
                None => notes.push("no food".into()),
            }
            if i.scaled {
                notes.push("scaled".into());
            }
            match i.shipped {
                Some(false) => notes.push("pantry".into()),
                None => notes.push("shipped unknown".into()),
                Some(true) => {}
            }
            if !i.allergens.is_empty() {
                notes.push(format!("allergens {}", i.allergens.join(", ")));
            }
            if let Some(note) = &i.note {
                notes.push(format!("note: {note}"));
            }
            if i.image.is_some() {
                notes.push("photo".into());
            }
            let amount = i.amount.as_deref().unwrap_or("to taste");
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!(" [{}]", notes.join("; "))
            };
            writeln!(out, "- {amount} {}{notes}", i.name).unwrap();
        }
        out.push('\n');
        for s in &v.steps {
            let flags: Vec<&str> = [
                (s.scaled, "scaled"),
                (s.check, "check"),
                (s.check_amounts, "check amounts"),
            ]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect();
            let photos = match s.images.len() {
                0 => String::new(),
                n => format!(" [{n} photos]"),
            };
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            };
            writeln!(out, "{}. {}{flags}{photos}", s.position, s.text).unwrap();
        }
    }
}

/// Serving counts as "2, 4".
fn list(yields: &[f64]) -> String {
    if yields.is_empty() {
        return "none".to_string();
    }
    yields
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Compare each file with its committed snapshot, or rewrite the snapshots when `UPDATE` is set. A snapshot with no
/// file to match (a dish that is gone) fails too, and is removed on update.
fn compare(actual: &BTreeMap<String, String>) {
    let dir = snapshot_dir();
    let update = std::env::var_os(UPDATE).is_some();
    let committed: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".txt"))
                .collect()
        })
        .unwrap_or_default();

    if update {
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in actual {
            // Bytes, so the file has LF line endings on Windows too.
            std::fs::write(dir.join(name), text.as_bytes()).unwrap();
        }
        for name in committed.iter().filter(|n| !actual.contains_key(*n)) {
            std::fs::remove_file(dir.join(name)).unwrap();
        }
        eprintln!(
            "rewrote the snapshots in {}; review them with git diff",
            dir.display()
        );
        return;
    }

    let mut failures = String::new();
    for (name, text) in actual {
        match std::fs::read(dir.join(name)) {
            Ok(bytes) => {
                // A checkout that turned LF into CRLF still matches.
                let expected = String::from_utf8(bytes).unwrap().replace("\r\n", "\n");
                if expected != *text {
                    failures.push_str(&diff(name, &expected, text));
                }
            }
            Err(_) => writeln!(failures, "snapshots/{name} is new:\n{text}").unwrap(),
        }
    }
    for name in committed.iter().filter(|n| !actual.contains_key(*n)) {
        writeln!(failures, "snapshots/{name} is no longer produced").unwrap();
    }
    assert!(
        failures.is_empty(),
        "the pipeline's output differs from fixtures/demo-bundle/snapshots/:\n\n{failures}\n\
         If the change is intended, rewrite the snapshots and review them with git diff:\n\
         {UPDATE}=1 cargo test -p mise-cli --test pipeline_snapshot"
    );
}

/// A unified diff from the snapshot to what the pipeline made. Like `git diff` on a source file, each hunk's header
/// names the section it falls in (the nearest heading above it), so a changed amount reads with its version and
/// serving count.
fn diff(name: &str, expected: &str, actual: &str) -> String {
    let diff = similar::TextDiff::from_lines(expected, actual);
    let lines: Vec<&str> = expected.lines().collect();
    let mut out = format!("--- snapshots/{name}\n+++ actual/{name}\n");
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        let start = hunk.ops().first().map_or(0, |op| op.old_range().start);
        let section = lines[..start.min(lines.len())]
            .iter()
            .rev()
            .find(|l| l.starts_with('#'))
            .copied()
            .unwrap_or("");
        writeln!(out, "{} {section}", hunk.header()).unwrap();
        for change in hunk.iter_changes() {
            write!(out, "{}{}", change.tag(), change.value()).unwrap();
            if change.missing_newline() {
                out.push('\n');
            }
        }
    }
    out
}

/// How long dropping the test database may take before it is given up and reported.
const DROP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A database made for this test and dropped with it, even when the test fails: the same as the API tests'
/// (`crates/mise-api/tests/common/mod.rs`).
struct Database {
    /// DATABASE_URL, which the test connects to as an administrator.
    admin_url: String,
    name: String,
}

impl Database {
    /// None when there is no database to use, outside CI; in CI that is a failure.
    async fn create() -> Option<Database> {
        dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env")).ok();
        let in_ci = std::env::var_os("CI").is_some();
        let admin_url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) if in_ci => panic!("DATABASE_URL must be set in CI"),
            Err(_) => return None,
        };
        let mut admin = match PgConnection::connect(&admin_url).await {
            Ok(conn) => conn,
            Err(e) if in_ci => panic!("can't connect to DATABASE_URL in CI: {e}"),
            Err(_) => return None,
        };
        let name = format!("mise_test_{}", uuid::Uuid::now_v7().simple());
        // The name is ours (a UUID), not input.
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&mut admin)
            .await
            .unwrap();
        admin.close().await.ok();
        Some(Database { admin_url, name })
    }

    /// The URL of this database: DATABASE_URL with the database name swapped.
    fn url(&self) -> String {
        let base = self.admin_url.rsplit_once('/').unwrap().0;
        format!("{base}/{}", self.name)
    }
}

impl Drop for Database {
    /// Dropped on a thread with its own runtime, since Drop runs outside async code (and while a failed test
    /// unwinds). A failure is reported, not raised: panicking here would abort the test run.
    fn drop(&mut self) {
        let (url, name) = (self.admin_url.clone(), self.name.clone());
        let dropped = std::thread::spawn(move || -> anyhow::Result<()> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                tokio::time::timeout(DROP_TIMEOUT, async {
                    let mut admin = PgConnection::connect(&url).await?;
                    // FORCE ends the connections the test's pool still holds.
                    sqlx::query(AssertSqlSafe(format!(
                        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
                    )))
                    .execute(&mut admin)
                    .await?;
                    admin.close().await?;
                    anyhow::Ok(())
                })
                .await
            })?
        })
        .join();
        match dropped {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("couldn't drop test database {}: {e:#}", self.name),
            Err(_) => eprintln!(
                "couldn't drop test database {}: the thread panicked",
                self.name
            ),
        }
    }
}
