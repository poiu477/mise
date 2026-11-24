//! Tests for reading exports: each format's fixture in `fixtures/import-files/` gives its expected records, packed
//! as the app packs it or unpacked as a folder; what isn't a recipe is passed over or said to be; and an export can't
//! unpack past the limits.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use super::*;

/// Set to rewrite each format's expected records from what the reader gives (then review the diff).
const UPDATE: &str = "MISE_UPDATE_SNAPSHOTS";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/import-files")
}

/// Each format's fixture folder, and the format its recipes are in.
const FORMATS: &[(&str, Format)] = &[
    ("schema-org", Format::SchemaOrg),
    ("nextcloud-cookbook", Format::NextcloudCookbook),
    ("mealie", Format::Mealie),
    ("tandoor", Format::Tandoor),
    ("paprika", Format::Paprika),
];

/// What the expected file holds for a recipe: where it was, its photo's place, and the record without `raw`.
fn summary(found: &Found) -> Value {
    let mut record = serde_json::to_value(&found.record).unwrap();
    record.as_object_mut().unwrap().remove("raw");
    json!({
        "format": found.format.name(),
        "path": found.path,
        "photo": found.photo.as_ref().map(|(path, _)| path),
        "record": record,
    })
}

#[test]
fn each_formats_fixture_gives_its_expected_records() {
    let update = std::env::var_os(UPDATE).is_some();
    let mut failures = Vec::new();
    for (folder, format) in FORMATS {
        let contents = read(&unpack_dir(&fixtures().join(folder)).unwrap())
            .unwrap_or_else(|e| panic!("{folder}: {e}"));
        assert!(
            contents.skipped.is_empty(),
            "{folder}: {:?}",
            contents.skipped
        );
        assert!(
            contents.recipes.iter().all(|f| f.format == *format),
            "{folder}"
        );
        let got = Value::Array(contents.recipes.iter().map(summary).collect());
        let path = fixtures().join("expected").join(format!("{folder}.json"));
        if update {
            let mut text = serde_json::to_string_pretty(&got).unwrap();
            text.push('\n');
            fs::write(&path, text).unwrap();
            continue;
        }
        let want: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if got != want {
            failures.push(format!(
                "{folder}: got\n{}",
                serde_json::to_string_pretty(&got).unwrap()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The files in a fixture folder, by path within it, `/` between folders.
fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        for entry in fs::read_dir(folder).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type().unwrap().is_dir() {
                stack.push((entry.path(), format!("{name}/")));
            } else {
                out.push((name, fs::read(entry.path()).unwrap()));
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

/// The fixture packed as its app exports it, with the name the export file has.
fn exported(folder: &str) -> (String, Vec<u8>) {
    let files = files_in(&fixtures().join(folder));
    match folder {
        // One recipe saved as a file.
        "schema-org" => {
            let [(name, bytes)] = files.as_slice() else {
                panic!("one schema.org file")
            };
            (name.clone(), bytes.clone())
        }
        // The Recipes folder, downloaded from Nextcloud as a zip.
        "nextcloud-cookbook" => ("Recipes.zip".into(), zipped(&files)),
        "mealie" => ("mealie_2026-11-24.zip".into(), zipped(&files)),
        // A zip of a zip for each recipe, each holding its folder's files.
        "tandoor" => {
            let mut inner: Vec<(String, Vec<u8>)> = Vec::new();
            let mut recipes: std::collections::BTreeMap<String, Vec<(String, Vec<u8>)>> =
                Default::default();
            for (path, bytes) in files {
                let (id, name) = path.split_once('/').unwrap();
                recipes
                    .entry(id.to_owned())
                    .or_default()
                    .push((name.to_owned(), bytes));
            }
            for (id, files) in recipes {
                inner.push((format!("{id}.zip"), zipped(&files)));
            }
            ("export_2026-11-24.zip".into(), zipped(&inner))
        }
        // A zip of gzipped recipes.
        "paprika" => {
            let recipes: Vec<(String, Vec<u8>)> = files
                .iter()
                .map(|(name, bytes)| (name.replace(".json", ".paprikarecipe"), gzipped(bytes)))
                .collect();
            ("My Recipes.paprikarecipes".into(), zipped(&recipes))
        }
        other => panic!("no export for {other}"),
    }
}

#[test]
fn each_export_reads_as_its_folder_does() {
    for (folder, _) in FORMATS {
        let from_folder = read(&unpack_dir(&fixtures().join(folder)).unwrap()).unwrap();
        let (name, bytes) = exported(folder);
        let from_export = read(&unpack(&name, bytes).unwrap()).unwrap();
        assert_eq!(
            from_folder.recipes.len(),
            from_export.recipes.len(),
            "{folder}"
        );
        for (a, b) in from_folder.recipes.iter().zip(&from_export.recipes) {
            assert_eq!(a.format, b.format, "{folder}");
            assert_eq!(a.record, b.record, "{folder}: {}", a.record.title);
            assert_eq!(
                a.photo.as_ref().map(|(_, bytes)| bytes),
                b.photo.as_ref().map(|(_, bytes)| bytes),
                "{folder}: {}",
                a.record.title
            );
        }
    }
}

#[test]
fn a_single_paprika_recipe_is_read_from_its_gzip() {
    let pudding = fs::read(fixtures().join("paprika/cardamom-rice-pudding.json")).unwrap();
    let contents =
        read(&unpack("Cardamom Rice Pudding.paprikarecipe", gzipped(&pudding)).unwrap()).unwrap();
    let [found] = contents.recipes.as_slice() else {
        panic!("one recipe")
    };
    assert_eq!(found.record.title, "Cardamom Rice Pudding");
    assert!(found.photo.is_some());
}

#[test]
fn a_record_keeps_the_apps_own_recipe_but_not_its_inline_photo() {
    let contents = read(&unpack_dir(&fixtures().join("paprika")).unwrap()).unwrap();
    let raw = &contents.recipes[0].record.raw;
    assert_eq!(raw["uid"], "8E2B6F1C-4D3A-4C5B-9A7E-2F1D0C3B4A59");
    assert_eq!(
        raw["notes"],
        "Thickens as it cools; loosen with a splash of milk."
    );
    assert!(raw.get("photo_data").is_none() && raw.get("photos").is_none());

    let contents = read(&unpack_dir(&fixtures().join("mealie")).unwrap()).unwrap();
    assert_eq!(contents.recipes[0].record.raw["slug"], "sheet-pan-gnocchi");
}

#[test]
fn a_photo_link_is_kept_with_the_recipe_but_not_fetched() {
    let contents = read(&unpack_dir(&fixtures().join("schema-org")).unwrap()).unwrap();
    let record = &contents.recipes[0].record;
    assert_eq!(record.hero_image, None);
    assert_eq!(
        record.meta.images,
        ["https://recipes.example/photos/miso-glazed-squash.jpg"]
    );
    assert!(contents.recipes[0].photo.is_none());
}

/// A Mealie recipe for the tests below, as JSON.
fn mealie_recipe(slug: &str) -> Vec<u8> {
    json!({
        "slug": slug, "name": format!("Recipe {slug}"),
        "recipe_ingredient": [{"note": "1 cup rice", "food": null, "quantity": 0}],
        "recipe_instructions": [{"text": "Cook the rice."}],
    })
    .to_string()
    .into_bytes()
}

#[test]
fn a_photo_beside_two_recipes_is_neithers() {
    let files = vec![
        ("both/a.json".to_owned(), mealie_recipe("a")),
        ("both/b.json".to_owned(), mealie_recipe("b")),
        (
            "both/image.jpg".to_owned(),
            fs::read(fixtures().join("tandoor/1/image.jpg")).unwrap(),
        ),
    ];
    let contents = read(&unpack("x.zip", zipped(&files)).unwrap()).unwrap();
    assert_eq!(contents.recipes.len(), 2);
    assert!(contents.recipes.iter().all(|f| f.photo.is_none()));
}

#[test]
fn what_isnt_a_recipe_is_passed_over_and_a_broken_recipe_is_skipped() {
    let files = vec![
        ("recipes/a/a.json".to_owned(), mealie_recipe("a")),
        // Settings an export keeps beside its recipes.
        ("settings.json".to_owned(), br#"{"theme": "dark"}"#.to_vec()),
        ("notes.txt".to_owned(), b"not looked at".to_vec()),
        // A Mac's copy of a resource fork.
        (
            "__MACOSX/recipes/a/._a.json".to_owned(),
            b"\0\x05\x16\x07".to_vec(),
        ),
        (
            "recipes/b/b.json".to_owned(),
            b"{\"slug\": \"b\", ".to_vec(),
        ),
        (
            "recipes/c/c.json".to_owned(),
            json!({"slug": "c", "recipe_ingredient": []})
                .to_string()
                .into_bytes(),
        ),
    ];
    let contents = read(&unpack("export.zip", zipped(&files)).unwrap()).unwrap();
    assert_eq!(contents.recipes.len(), 1);
    assert_eq!(contents.recipes[0].record.title, "Recipe a");
    let skipped: Vec<(&str, &str)> = contents
        .skipped
        .iter()
        .map(|s| (s.path.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert_eq!(skipped[0].0, "recipes/b/b.json");
    assert!(skipped[0].1.starts_with("not valid JSON"), "{skipped:?}");
    assert_eq!(skipped[1], ("recipes/c/c.json", "the recipe has no name"));
}

#[test]
fn a_file_that_isnt_a_recipe_says_why() {
    let error = |name: &str, bytes: &[u8]| {
        read(&unpack(name, bytes.to_vec()).unwrap())
            .unwrap_err()
            .to_string()
    };
    let not_json = error("photo.json", b"\x89PNG\r\n");
    assert!(
        not_json.contains("holds no recipe mise reads"),
        "{not_json}"
    );
    assert!(not_json.contains("not valid JSON"), "{not_json}");
    let other_json = error("package.json", br#"{"name": "left-pad"}"#);
    assert!(
        other_json.ends_with("it is JSON, but not a recipe in any of those formats"),
        "{other_json}"
    );
    let empty_zip = error("empty.zip", &zipped(&[]));
    assert!(
        empty_zip.contains("holds no recipe mise reads"),
        "{empty_zip}"
    );
    let broken = unpack("broken.zip", b"PK\x03\x04 and then nothing".to_vec()).unwrap_err();
    assert!(matches!(broken, FileError::Unreadable(_)), "{broken}");
}

#[test]
fn zips_are_opened_two_deep_and_no_deeper() {
    let recipe = vec![("r.json".to_owned(), mealie_recipe("r"))];
    let two = zipped(&[("inner.zip".to_owned(), zipped(&recipe))]);
    assert_eq!(
        read(&unpack("two.zip", two.clone()).unwrap())
            .unwrap()
            .recipes
            .len(),
        1
    );
    let three = zipped(&[("middle.zip".to_owned(), two)]);
    assert!(matches!(
        read(&unpack("three.zip", three).unwrap()),
        Err(FileError::NoRecipes(_))
    ));
}

#[test]
fn an_entry_unpacks_no_larger_than_the_limit() {
    // 26 MB of zeros packs into a few kilobytes: the size is checked as it unpacks, whatever the zip claims.
    let big = vec![0u8; MAX_ENTRY_BYTES as usize + 1];
    let bomb = zipped(&[("huge.json".to_owned(), big.clone())]);
    assert!(bomb.len() < 100_000);
    let error = unpack("bomb.zip", bomb).unwrap_err();
    assert!(matches!(error, FileError::TooLarge(_)), "{error}");
    assert!(
        error
            .to_string()
            .contains("huge.json unpacks to more than 25 MB"),
        "{error}"
    );

    let error = read(&unpack("bomb.paprikarecipe", gzipped(&big)).unwrap()).unwrap_err();
    assert!(
        error.to_string().contains("unpacks to more than 25 MB"),
        "{error}"
    );
}

#[test]
fn too_many_files_are_refused() {
    let mut budget = Budget::default();
    for _ in 0..MAX_ENTRIES {
        budget.take(0).unwrap();
    }
    assert!(matches!(budget.take(0), Err(FileError::TooLarge(_))));
    let mut budget = Budget::default();
    assert!(matches!(
        budget.take(MAX_UNPACKED_BYTES + 1),
        Err(FileError::TooLarge(_))
    ));
}

#[test]
fn amounts_are_written_as_a_cook_writes_them() {
    assert_eq!(amount_text(2.0), "2");
    assert_eq!(amount_text(1.25), "1.25");
    assert_eq!(amount_text(1.0 / 3.0), "0.333");
    assert_eq!(amount_text(0.5), "0.5");
}
