//! Tests for reading schema.org Recipe data: the sample pages in `fixtures/web-pages/`, the rules for yields,
//! times and steps, and a check that no input makes the reader panic.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use url::Url;

use super::*;

/// Set to rewrite each page's expected records from what the reader gives (then review the diff, as for the
/// pipeline snapshots).
const UPDATE: &str = "MISE_UPDATE_SNAPSHOTS";

/// The phase gate asks for this many sample pages.
const SAMPLE_PAGES: usize = 20;

fn pages_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/web-pages")
}

/// The address each sample page is read as having come from.
fn fetched_from(name: &str) -> Url {
    Url::parse(&format!("https://fixtures.example/{name}.html")).unwrap()
}

/// The sample pages that hold a recipe, by file stem.
fn sample_pages() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(pages_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "html"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .filter(|name| name != "no-recipe")
        .collect();
    names.sort();
    names
}

/// A record as its expected file holds it: everything but `raw`, which is the page's own Recipe object.
fn without_raw(record: &Record) -> Value {
    let mut value = serde_json::to_value(record).unwrap();
    value.as_object_mut().unwrap().remove("raw");
    value
}

#[test]
fn every_sample_page_gives_its_expected_records() {
    let update = std::env::var_os(UPDATE).is_some();
    let names = sample_pages();
    assert_eq!(names.len(), SAMPLE_PAGES, "sample pages: {names:?}");
    let mut failures = Vec::new();
    for name in &names {
        let page = fs::read_to_string(pages_dir().join(format!("{name}.html"))).unwrap();
        let records = read_page(&page, Some(&fetched_from(name)))
            .unwrap_or_else(|e| panic!("{name}.html: {e}"));
        for record in &records {
            assert!(
                is_recipe(record.raw.as_object().unwrap()),
                "{name}.html: raw is not the Recipe"
            );
        }
        let actual = Value::Array(records.iter().map(without_raw).collect());
        let path = pages_dir().join(format!("{name}.json"));
        if update {
            let mut text = serde_json::to_string_pretty(&actual).unwrap();
            text.push('\n');
            fs::write(&path, text.as_bytes()).unwrap();
            continue;
        }
        let expected: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap_or_default())
            .unwrap_or(Value::Null);
        if actual != expected {
            failures.push(format!(
                "{name}.html\n--- expected ({name}.json)\n{}\n--- read\n{}",
                serde_json::to_string_pretty(&expected).unwrap(),
                serde_json::to_string_pretty(&actual).unwrap()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} pages differ from their expected records; if the change is meant, run \
         {UPDATE}=1 cargo test -p mise-ingest schema_org and review the diff\n\n{}",
        failures.len(),
        names.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_expected_file_is_a_line_the_bundle_importer_reads() {
    // The expected files leave out `raw`; with it back they must still read as records.
    for name in sample_pages() {
        let text = fs::read_to_string(pages_dir().join(format!("{name}.json"))).unwrap();
        let records: Vec<Value> = serde_json::from_str(&text).unwrap();
        for mut record in records {
            record["raw"] = json!({});
            let record: Record =
                serde_json::from_value(record).unwrap_or_else(|e| panic!("{name}.json: {e}"));
            assert!(
                !record.title.is_empty() && !record.ingredients.is_empty(),
                "{name}.json"
            );
        }
    }
}

#[test]
fn a_page_with_no_recipe_says_so() {
    let page = fs::read_to_string(pages_dir().join("no-recipe.html")).unwrap();
    let error = read_page(&page, Some(&fetched_from("no-recipe"))).unwrap_err();
    assert!(matches!(
        error,
        ReadError::NoRecipe {
            unreadable_blocks: 0
        }
    ));
    assert_eq!(
        error.to_string(),
        "no recipe found: the page has no schema.org Recipe data (JSON-LD or microdata)"
    );
}

#[test]
fn a_page_whose_only_recipe_block_is_broken_counts_it() {
    let page =
        r#"<script type="application/ld+json">{"@type": "Recipe", "name": "Toast",</script>"#;
    let error = read_page(page, None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "no recipe found: the page has no schema.org Recipe data that could be read \
         (1 JSON-LD block is not valid JSON)"
    );
}

#[test]
fn an_empty_or_plain_page_has_no_recipe() {
    for page in [
        "",
        "just some text",
        "<html><body><p>Hello</p></body></html>",
    ] {
        assert!(
            matches!(read_page(page, None), Err(ReadError::NoRecipe { .. })),
            "{page:?}"
        );
    }
}

#[test]
fn a_recipe_with_no_name_takes_the_page_title_or_fails() {
    let block = r#"<script type="application/ld+json">{"@type": "Recipe", "recipeIngredient": ["1 egg"]}</script>"#;
    let with_title = format!("<title>Boiled &amp; Peeled Egg</title>{block}");
    assert_eq!(
        read_page(&with_title, None).unwrap()[0].title,
        "Boiled & Peeled Egg"
    );
    assert!(matches!(read_page(block, None), Err(ReadError::Untitled)));
}

#[test]
fn json_documents_are_read_like_page_blocks() {
    let doc = r#"{"@context": "https://schema.org", "@type": "Recipe", "name": "Toast",
                  "image": "toast.jpg", "recipeIngredient": ["1 slice bread"],
                  "recipeInstructions": "Toast the bread."}"#;
    let url = Url::parse("https://files.example/exports/toast.json").unwrap();
    let records = read_json(doc, Some(&url)).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].url.as_deref(),
        Some("https://files.example/exports/toast.json")
    );
    assert_eq!(
        records[0].hero_image.as_deref(),
        Some("https://files.example/exports/toast.jpg")
    );
    assert_eq!(records[0].steps[0].text, "Toast the bread.");

    let without_url = read_json(doc, None).unwrap();
    assert!(without_url[0].external_id.starts_with("sha256:"));
    assert_eq!(
        without_url[0].hero_image, None,
        "a relative image needs an address to resolve against"
    );

    assert!(matches!(
        read_json("{not json", None),
        Err(ReadError::Json(_))
    ));
    assert!(matches!(
        read_json(r#"{"@type": "Person"}"#, None),
        Err(ReadError::NoRecipe { .. })
    ));
}

#[test]
fn yields_count_servings_and_take_the_lower_end_of_a_range() {
    let cases = [
        ("4", Some(4.0)),
        ("4 servings", Some(4.0)),
        ("Serves 4", Some(4.0)),
        ("4-6 servings", Some(4.0)),
        ("4–6", Some(4.0)),
        ("Serves 4 to 6", Some(4.0)),
        ("6 people", Some(6.0)),
        ("2.5", Some(2.5)),
        ("Makes 24 cookies", None),
        ("1 loaf", None),
        ("1 jar", None),
        ("0", None),
        ("a few", None),
        ("5000 servings", None),
    ];
    for (text, expected) in cases {
        assert_eq!(servings(text), expected, "{text:?}");
    }
}

#[test]
fn times_become_hours_and_minutes() {
    let cases = [
        ("PT20M", Some("PT20M")),
        ("PT0H15M", Some("PT15M")),
        ("PT90M", Some("PT1H30M")),
        ("P0DT1H30M", Some("PT1H30M")),
        ("PT2H", Some("PT2H")),
        ("PT1.5H", Some("PT1H30M")),
        ("pt45m", Some("PT45M")),
        ("PT600S", Some("PT10M")),
        ("P1D", Some("PT24H")),
        ("1 hr 30 mins", Some("PT1H30M")),
        ("45 minutes", Some("PT45M")),
        ("1h20m", Some("PT1H20M")),
        ("2 hours and 5 minutes", Some("PT2H5M")),
        ("PT0M", None),
        ("P0DT0H0M", None),
        ("PT", None),
        ("P1M", None),
        ("30", None),
        ("soon", None),
        ("", None),
    ];
    for (text, expected) in cases {
        assert_eq!(duration(text).as_deref(), expected, "{text:?}");
    }
}

#[test]
fn numbering_written_into_plain_steps_comes_off() {
    assert_eq!(strip_numbering("1. Preheat the oven."), "Preheat the oven.");
    assert_eq!(strip_numbering("12) Serve."), "Serve.");
    assert_eq!(strip_numbering("Step 3: Rest."), "Rest.");
    assert_eq!(strip_numbering("2 eggs, beaten in."), "2 eggs, beaten in.");
    assert_eq!(strip_numbering("350°F oven."), "350°F oven.");
    assert_eq!(
        strip_numbering("1.5 cups stock go in."),
        "1.5 cups stock go in."
    );
}

#[test]
fn lists_split_at_commas_without_repeats() {
    let refs = Refs::default();
    assert_eq!(
        list(&json!("Dinner, Pasta, dinner"), &refs),
        ["Dinner", "Pasta"]
    );
    assert_eq!(
        list(&json!(["Soup", "Quick, Easy"]), &refs),
        ["Soup", "Quick", "Easy"]
    );
}

#[test]
fn deep_nesting_is_refused_quickly_and_nested_items_are_cut_off() {
    // Past the limit: refused, without reading the rest of the page.
    let deep = "<div>".repeat(200_000);
    let started = std::time::Instant::now();
    let error = read_page(&deep, None).unwrap_err();
    assert!(matches!(error, ReadError::TooDeep));
    assert_eq!(
        error.to_string(),
        "the page nests its elements more than 512 deep, deeper than a browser reads"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "took {:?}",
        started.elapsed()
    );

    // Within it: microdata items nested hundreds deep are followed only a few levels down.
    let depth = 400;
    let page = format!(
        "{}<span itemprop=name>Deep</span>{}",
        r#"<div itemscope itemtype="https://schema.org/Recipe" itemprop="x">"#.repeat(depth),
        "</div>".repeat(depth)
    );
    assert_eq!(read_page(&page, None).unwrap()[0].title, "Deep");

    // JSON nested past the search depth is not searched (serde_json itself stops at 128 levels).
    let nested = format!(
        "{}{}{}",
        "[".repeat(100),
        r#"{"@type": "Recipe", "name": "Deep"}"#,
        "]".repeat(100)
    );
    assert!(matches!(
        read_json(&nested, None),
        Err(ReadError::NoRecipe { .. })
    ));
}

/// A small, fixed pseudo-random sequence (xorshift), so a failure repeats.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn no_page_makes_the_reader_panic() {
    // Every sample page cut short at many points, as a slow or capped download would leave it.
    let mut pages: Vec<String> = sample_pages()
        .iter()
        .chain(["no-recipe".to_owned()].iter())
        .map(|name| fs::read_to_string(pages_dir().join(format!("{name}.html"))).unwrap())
        .collect();
    let url = fetched_from("cut");
    for page in &pages {
        for cut in (0..page.len())
            .step_by(37)
            .filter(|&i| page.is_char_boundary(i))
        {
            let _ = read_page(&page[..cut], Some(&url));
        }
    }

    // Random soup of the pieces recipe markup is made of.
    const PIECES: &[&str] = &[
        "<",
        ">",
        "</",
        "/>",
        "div",
        "li",
        "ul",
        "script",
        "template",
        "table",
        "td",
        "p",
        "b",
        "i",
        " ",
        "\n",
        "\"",
        "'",
        "=",
        r#" type="application/ld+json""#,
        r#" itemscope"#,
        r#" itemtype="https://schema.org/Recipe""#,
        r#" itemtype="https://schema.org/HowToStep""#,
        r#" itemprop="recipeIngredient""#,
        r#" itemprop="recipeInstructions""#,
        r#" itemprop="name""#,
        r#" itemprop="image""#,
        r#" content="PT"#,
        " src=",
        " href=",
        "{",
        "}",
        "[",
        "]",
        ":",
        ",",
        r#""@type""#,
        r#""Recipe""#,
        r#""HowToSection""#,
        r#""HowToStep""#,
        r#""itemListElement""#,
        r#""@graph""#,
        r#""@id""#,
        "\"#a\"",
        r#""name""#,
        r#""image""#,
        r#""recipeYield""#,
        r#""recipeInstructions""#,
        r#""recipeIngredient""#,
        r#""prepTime""#,
        "\"PT1H",
        "4-6",
        "Serves",
        "0",
        "9",
        ".",
        "&amp;",
        "&#",
        "&frac12;",
        "<!--",
        "-->",
        "<![CDATA[",
        "]]>",
        "\u{0}",
        "\u{feff}",
        "é",
        "½",
        "<base href=\"//x\">",
        "<link rel=canonical href=\"::\">",
        "javascript:",
        "data:",
    ];
    let mut noise = Noise(0x5eed_cafe_f00d_d00d);
    for _ in 0..3000 {
        let len = noise.below(200);
        let page: String = (0..len)
            .map(|_| PIECES[noise.below(PIECES.len())])
            .collect();
        let _ = read_page(&page, Some(&url));
        let _ = read_json(&page, Some(&url));
    }

    // Sample pages with random pieces dropped in at random places.
    for page in &mut pages {
        for _ in 0..40 {
            let at = noise.below(page.len());
            if page.is_char_boundary(at) {
                page.insert_str(at, PIECES[noise.below(PIECES.len())]);
            }
        }
        let _ = read_page(page, Some(&url));
    }
}
