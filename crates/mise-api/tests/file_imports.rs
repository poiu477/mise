//! Importing the files other recipe apps export: each format's fixture (`fixtures/import-files`, written for mise),
//! packed as its app packs it, sent to the API and made into versions the importing household alone sees, with the
//! photos the file keeps; an unchanged file again changes nothing and a changed recipe becomes a new version; what
//! can't be read is listed while the rest is imported; and what isn't an export is refused.

#[macro_use]
mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{StatusCode, header};
use common::{As, TestApp};
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/import-files")
}

/// The files in a fixture folder, by path within it.
fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for entry in std::fs::read_dir(folder).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type().unwrap().is_dir() {
                stack.push((entry.path(), format!("{name}/")));
            } else {
                out.push((name, std::fs::read(entry.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn zipped(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(name.as_str(), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn gzipped(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

/// The Paprika fixture as Paprika exports it: a zip of gzipped recipes.
fn paprika_export(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let recipes: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(name, bytes)| (name.replace(".json", ".paprikarecipe"), gzipped(bytes)))
        .collect();
    zipped(&recipes)
}

/// A format's fixture as its app exports it, and the file's name.
fn exported(folder: &str) -> (&'static str, Vec<u8>) {
    let files = files_in(&fixtures().join(folder));
    match folder {
        "schema-org" => ("miso-glazed-squash.json", files[0].1.clone()),
        "nextcloud-cookbook" => ("Recipes.zip", zipped(&files)),
        "mealie" => ("mealie_2026-11-24.zip", zipped(&files)),
        "tandoor" => {
            let mut recipes: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
            for (path, bytes) in files {
                let (id, name) = path.split_once('/').unwrap();
                recipes
                    .entry(id.to_owned())
                    .or_default()
                    .push((name.to_owned(), bytes));
            }
            let inner: Vec<(String, Vec<u8>)> = recipes
                .into_iter()
                .map(|(id, files)| (format!("{id}.zip"), zipped(&files)))
                .collect();
            ("export_2026-11-24.zip", zipped(&inner))
        }
        "paprika" => ("My Recipes.paprikarecipes", paprika_export(&files)),
        other => panic!("no export for {other}"),
    }
}

/// The demo install with the owner signed in, and a second household signed in too.
struct Kitchen {
    app: TestApp,
    home: String,
    next_door: String,
}

impl Kitchen {
    async fn new() -> Option<Kitchen> {
        let app = TestApp::with_demo().await?;
        let home = app.setup_owner().await;
        let household = app
            .json(
                "POST",
                "/api/v1/households",
                &home,
                Some(json!({"name": "Next door"})),
            )
            .await;
        app.json(
            "POST",
            "/api/v1/users",
            &home,
            Some(json!({"username": "neighbour", "password": "another long password", "household_id": household["id"]})),
        )
        .await;
        let next_door = app.sign_in("neighbour", "another long password").await;
        Some(Kitchen {
            app,
            home,
            next_door,
        })
    }

    /// Sends a file to import as this browser; the status and body.
    async fn import(&self, cookie: &str, name: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
        let req = TestApp::request(
            "POST",
            &format!("/api/v1/imports/file?name={}", name.replace(' ', "%20")),
            As::Cookie(cookie),
        )
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
        let reply = self.app.read(req).await;
        (reply.status, reply.body)
    }

    /// A version as this browser sees it.
    async fn version(&self, cookie: &str, id: &str) -> (StatusCode, Value) {
        let reply = self
            .app
            .send(
                "GET",
                &format!("/api/v1/variants/{id}"),
                As::Cookie(cookie),
                None,
            )
            .await;
        (reply.status, reply.body)
    }
}

/// A recipe a fixture imports to: its title, and whether it has a photo.
type Recipe = (&'static str, bool);

/// What each format's fixture imports to: the fixture's folder, the format's name, and its recipes.
const EXPECTED: &[(&str, &str, &[Recipe])] = &[
    (
        "schema-org",
        "schema.org JSON",
        &[("Miso-Glazed Squash", false)],
    ),
    (
        "nextcloud-cookbook",
        "Nextcloud Cookbook",
        &[
            ("Lemon Barley Salad", true),
            ("Spiced Red Lentil Soup", false),
        ],
    ),
    (
        "mealie",
        "Mealie",
        &[("Sheet-Pan Gnocchi with Peppers", true)],
    ),
    ("tandoor", "Tandoor", &[("Brown Butter Cornbread", true)]),
    (
        "paprika",
        "Paprika",
        &[
            ("Cardamom Rice Pudding", true),
            ("Quick Pickled Red Onions", false),
        ],
    ),
];

#[tokio::test]
async fn each_format_imports_as_versions_of_the_household_s_own() {
    let k = test_app!(Kitchen::new());
    for (folder, format, recipes) in EXPECTED {
        let (name, bytes) = exported(folder);
        let (status, body) = k.import(&k.home, name, bytes).await;
        assert_eq!(status, StatusCode::OK, "{folder}: {body}");
        assert_eq!(body["formats"], json!([format]), "{folder}");
        assert_eq!(body["skipped"], json!([]), "{folder}");
        let got = body["recipes"].as_array().unwrap();
        assert_eq!(got.len(), recipes.len(), "{folder}: {body}");
        for (recipe, (title, has_photo)) in got.iter().zip(recipes.iter()) {
            assert_eq!(recipe["title"], *title, "{folder}");
            assert_eq!(recipe["unchanged"], false, "{folder}: {title}");
            assert_eq!(recipe["photo_problem"], Value::Null, "{folder}: {title}");
            let id = recipe["variant_id"].as_str().unwrap();
            let (status, version) = k.version(&k.home, id).await;
            assert_eq!(status, StatusCode::OK, "{folder}: {title}");
            assert_eq!(version["title"], *title);
            assert_eq!(
                version["image"].is_object(),
                *has_photo,
                "{folder}: {title}: {}",
                version["image"]
            );
            assert!(
                !version["ingredients"].as_array().unwrap().is_empty()
                    && !version["steps"].as_array().unwrap().is_empty(),
                "{folder}: {title}"
            );
            // Nobody else sees it.
            let (status, _) = k.version(&k.next_door, id).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{folder}: {title}");
        }
    }

    // Each format is a source of its own, every record the household's.
    let sources: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.key, count(*) FROM source_records sr JOIN sources s ON s.id = sr.source_id
         WHERE s.kind = 'file' AND sr.household_id IS NOT NULL GROUP BY s.key ORDER BY s.key",
    )
    .fetch_all(&k.app.pool)
    .await
    .unwrap();
    assert_eq!(
        sources,
        [
            ("file:mealie".to_owned(), 1),
            ("file:nextcloud-cookbook".to_owned(), 2),
            ("file:paprika".to_owned(), 2),
            ("file:schema-org".to_owned(), 1),
            ("file:tandoor".to_owned(), 1),
        ]
    );
}

#[tokio::test]
async fn amounts_from_a_file_scale_as_any_recipe_s_do() {
    let k = test_app!(Kitchen::new());
    let (name, bytes) = exported("tandoor");
    let (_, body) = k.import(&k.home, name, bytes).await;
    let id = body["recipes"][0]["variant_id"].as_str().unwrap();
    // Written for 8; at 4, half of each.
    let version = k
        .app
        .json(
            "GET",
            &format!("/api/v1/variants/{id}?servings=4"),
            &k.home,
            None,
        )
        .await;
    let amounts: BTreeMap<String, String> = version["ingredients"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap().to_owned(),
                i["amount"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    assert_eq!(amounts["cornmeal"], "½ cup", "{amounts:?}");
    assert_eq!(amounts["eggs"], "1", "{amounts:?}");
}

#[tokio::test]
async fn an_unchanged_file_again_changes_nothing_and_a_changed_recipe_is_a_new_version() {
    let k = test_app!(Kitchen::new());
    let files = files_in(&fixtures().join("paprika"));
    let (_, first) = k
        .import(&k.home, "My Recipes.paprikarecipes", paprika_export(&files))
        .await;
    let count = |table: &'static str| {
        let pool = k.app.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table}"
            )))
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let before = (count("variants").await, count("media").await);

    let (status, again) = k
        .import(&k.home, "My Recipes.paprikarecipes", paprika_export(&files))
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    for (a, b) in first["recipes"]
        .as_array()
        .unwrap()
        .iter()
        .zip(again["recipes"].as_array().unwrap())
    {
        assert_eq!(b["unchanged"], true);
        assert_eq!(a["variant_id"], b["variant_id"]);
    }
    assert_eq!((count("variants").await, count("media").await), before);

    // The pudding changed in Paprika since: a new version, whose parent is the one imported before.
    let mut changed = files.clone();
    let pudding: &mut (String, Vec<u8>) = changed
        .iter_mut()
        .find(|(name, _)| name.starts_with("cardamom"))
        .unwrap();
    let mut recipe: Value = serde_json::from_slice(&pudding.1).unwrap();
    recipe["servings"] = json!("6 servings");
    pudding.1 = recipe.to_string().into_bytes();
    let (_, third) = k
        .import(
            &k.home,
            "My Recipes.paprikarecipes",
            paprika_export(&changed),
        )
        .await;
    let (old, new) = (&first["recipes"][0], &third["recipes"][0]);
    assert_eq!(new["unchanged"], false);
    assert_ne!(new["variant_id"], old["variant_id"]);
    assert_eq!(third["recipes"][1]["unchanged"], true);
    let parent: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT parent_variant_id FROM variants WHERE id = $1::uuid")
            .bind(new["variant_id"].as_str().unwrap())
            .fetch_one(&k.app.pool)
            .await
            .unwrap();
    assert_eq!(
        parent.map(|p| p.to_string()).as_deref(),
        old["variant_id"].as_str()
    );

    // Next door importing the same file gets versions of its own.
    let (_, theirs) = k
        .import(
            &k.next_door,
            "My Recipes.paprikarecipes",
            paprika_export(&files),
        )
        .await;
    assert_eq!(theirs["recipes"][1]["unchanged"], false);
    assert_ne!(
        theirs["recipes"][1]["variant_id"],
        first["recipes"][1]["variant_id"]
    );
}

#[tokio::test]
async fn what_can_t_be_read_is_listed_and_the_rest_imported() {
    let k = test_app!(Kitchen::new());
    let mut files = files_in(&fixtures().join("mealie"));
    files.push((
        "recipes/broken/broken.json".to_owned(),
        b"{\"slug\": ".to_vec(),
    ));
    let (status, body) = k.import(&k.home, "mealie.zip", zipped(&files)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recipes"].as_array().unwrap().len(), 1);
    assert_eq!(body["skipped"][0]["path"], "recipes/broken/broken.json");
    assert!(
        body["skipped"][0]["reason"]
            .as_str()
            .unwrap()
            .starts_with("not valid JSON"),
        "{body}"
    );
}

#[tokio::test]
async fn a_photo_the_file_holds_that_isn_t_one_is_said_and_the_recipe_kept() {
    let k = test_app!(Kitchen::new());
    let mut files = files_in(&fixtures().join("tandoor"));
    for (path, bytes) in &mut files {
        if path.ends_with("image.jpg") {
            *bytes = b"not a photo at all".to_vec();
        }
    }
    let inner = zipped(
        &files
            .iter()
            .map(|(p, b)| (p.trim_start_matches("1/").to_owned(), b.clone()))
            .collect::<Vec<_>>(),
    );
    let (status, body) = k
        .import(
            &k.home,
            "export.zip",
            zipped(&[("1.zip".to_owned(), inner)]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["recipes"][0]["photo_problem"],
        "its photo couldn't be kept: it isn't an image mise reads"
    );
}

#[tokio::test]
async fn what_isn_t_an_export_is_refused() {
    let k = test_app!(Kitchen::new());
    let (status, body) = k
        .import(&k.home, "package.json", br#"{"name": "left-pad"}"#.to_vec())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("it holds no recipe mise reads"),
        "{body}"
    );

    // Sent as a form on another site could send it: refused before it is read.
    let req = TestApp::request("POST", "/api/v1/imports/file", As::Cookie(&k.home))
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=x")
        .body(Body::from(exported("schema-org").1))
        .unwrap();
    assert_eq!(k.app.read(req).await.status, StatusCode::BAD_REQUEST);
    let found = k
        .app
        .json("GET", "/api/v1/dishes?q=squash", &k.home, None)
        .await;
    assert_eq!(found["total"], 0);
}
