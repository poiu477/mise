//! Read schema.org Recipe data from a web page or a JSON-LD document into records shaped like a seed bundle's
//! variant lines (docs/seed-bundle.md), ready for the shared import stages.
//!
//! JSON-LD comes first: every `<script type="application/ld+json">` block, looked through for objects whose
//! `@type` includes `Recipe` (at the top, in an array, in `@graph`, or inside another item). Microdata
//! (`itemscope itemtype="https://schema.org/Recipe"`) is read only when no block holds a recipe. Reading is pure:
//! it takes text and fetches nothing.

mod html;

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use url::Url;

use html::{Html, MAX_NESTING, last_segment};

/// JSON nested deeper than this is not searched for recipes; pages nest three or four levels.
const MAX_DEPTH: usize = 32;

/// A yield above this is a typo or a count of something other than servings.
const MAX_SERVINGS: f64 = 1000.0;

/// One recipe read from schema.org data, with the fields of a seed bundle's variant line (`mise-seed/1`) that a
/// web page can fill. Serialized, it is a valid line of `variants.ndjson`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Stable id within the `web` source: the recipe's URL, or `sha256:` and a hash of its data when it has
    /// none. When one page holds several recipes at one URL, the second and later ones end in `#2`, `#3`.
    pub external_id: String,
    /// Where the recipe lives, without any `#fragment`: the recipe's own `url`, its `mainEntityOfPage`, the page's
    /// canonical link, `og:url`, or the address it was read from, in that order.
    pub url: Option<String>,
    /// The recipe's `name` ("Weeknight Tomato Pasta"), or the page's title when the data has none.
    pub title: String,
    /// The recipe's `description`, paragraphs separated by a blank line.
    pub description: Option<String>,
    /// Times, the yield as written, categories and images.
    pub meta: Meta,
    /// Serving counts the amounts are written for: `[4]` from "4 servings", "Serves 4" or "4-6 servings" (the
    /// lower end). Empty when the yield counts something else ("Makes 24 cookies").
    pub yields: Vec<f64>,
    /// Ingredient lines as the page writes them ("2 cups flour"), in order; amounts are read from them later.
    pub ingredients: Vec<Ingredient>,
    /// The method, one step per instruction, numbered from 1.
    pub steps: Vec<Step>,
    /// The first image URL: the photo to show for the recipe.
    pub hero_image: Option<String>,
    /// The Recipe object as the page gave it (microdata as the equivalent JSON), kept for re-processing.
    pub raw: Value,
}

/// What a recipe says beyond its lines and steps. Times are ISO 8601 durations in hours and minutes ("PT1H30M").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// `prepTime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prep_time: Option<String>,
    /// `cookTime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cook_time: Option<String>,
    /// `totalTime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_time: Option<String>,
    /// `recipeYield` as written, when it says more than a number ("4-6 servings", "Makes 24 cookies").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yield_text: Option<String>,
    /// The author's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// `recipeCuisine` ("Italian").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cuisines: Vec<String>,
    /// `recipeCategory` ("Dinner", "Side dish").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// `keywords`, split at commas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Every image URL of the recipe, absolute, the hero first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// One ingredient line, as a seed bundle's ingredient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ingredient {
    /// The line's position from 1 ("1", "2"), unique within the record.
    pub key: String,
    /// The line as the page writes it, entities decoded and spaces tidied: "2 cups flour, sifted".
    pub name: String,
}

/// One step of the method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// Position from 1.
    pub index: i32,
    /// The instruction, on one line.
    pub text: String,
    /// The name of the `HowToSection` the step is in ("For the sauce").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Absolute URLs of the step's own photos.
    #[serde(default)]
    pub images: Vec<String>,
}

/// Why nothing could be read.
#[derive(Debug)]
pub enum ReadError {
    /// No JSON-LD block or microdata item has a Recipe.
    NoRecipe {
        /// JSON-LD blocks on the page that weren't valid JSON, so may have held one.
        unreadable_blocks: usize,
    },
    /// Every Recipe found has no name, and the page has no title to stand in.
    Untitled,
    /// The page nests its elements deeper than a browser reads, so it isn't read: parsing it would take very long.
    TooDeep,
    /// The document given to [`read_json`] isn't JSON.
    Json(serde_json::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::NoRecipe {
                unreadable_blocks: 0,
            } => {
                write!(
                    f,
                    "no recipe found: the page has no schema.org Recipe data (JSON-LD or microdata)"
                )
            }
            ReadError::NoRecipe { unreadable_blocks } => write!(
                f,
                "no recipe found: the page has no schema.org Recipe data that could be read \
                 ({unreadable_blocks} JSON-LD block{} not valid JSON)",
                if *unreadable_blocks == 1 {
                    " is"
                } else {
                    "s are"
                }
            ),
            ReadError::Untitled => write!(f, "the recipe has no name"),
            ReadError::TooDeep => write!(
                f,
                "the page nests its elements more than {MAX_NESTING} deep, deeper than a browser reads"
            ),
            ReadError::Json(e) => write!(f, "not valid JSON: {e}"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReadError::Json(e) => Some(e),
            _ => None,
        }
    }
}

/// Reads every recipe on a web page, in page order. `page_url` is the address the page came from; relative links
/// are resolved against it (or the page's `<base>`), and it is the recipe's URL when the page names none.
///
/// Fails with [`ReadError::NoRecipe`] when the page has no Recipe data, and [`ReadError::TooDeep`] when its elements
/// nest past what a browser reads.
pub fn read_page(page: &str, page_url: Option<&Url>) -> Result<Vec<Record>, ReadError> {
    let html = Html::page(page).map_err(|_| ReadError::TooDeep)?;
    let base = match html.base_href() {
        Some(href) => join(page_url, href).or_else(|| page_url.cloned()),
        None => page_url.cloned(),
    };
    let page_link = html
        .head_value(Some("canonical"), None)
        .and_then(|href| join(base.as_ref(), &href))
        .or_else(|| {
            html.head_value(None, Some("og:url"))
                .and_then(|href| join(base.as_ref(), &href))
        })
        .or_else(|| page_url.cloned());
    let page_title = html
        .head_value(None, Some("og:title"))
        .or_else(|| html.title())
        .and_then(|t| line(&t));

    let mut documents = Vec::new();
    let mut unreadable = 0;
    for block in html.json_ld_blocks() {
        match parse_block(&block) {
            Some(doc) => documents.push(doc),
            None => unreadable += 1,
        }
    }
    let refs: Vec<Refs> = documents.iter().map(Refs::of).collect();
    let no_refs = Refs::default();
    let mut found: Vec<(Value, &Refs)> = Vec::new();
    for (doc, refs) in documents.iter().zip(&refs) {
        let mut recipes = Vec::new();
        find_recipes(doc, 0, &mut recipes);
        found.extend(recipes.into_iter().map(|r| (r.clone(), refs)));
    }
    if found.is_empty() {
        found = html
            .microdata_items(&["Recipe"])
            .into_iter()
            .map(|item| (item, &no_refs))
            .collect();
    }
    if found.is_empty() {
        return Err(ReadError::NoRecipe {
            unreadable_blocks: unreadable,
        });
    }
    let context = Context {
        base: base.as_ref(),
        page_link: page_link.as_ref(),
        page_title: page_title.as_deref(),
    };
    records(found, &context)
}

/// Reads every recipe in a JSON-LD document (a schema.org Recipe export, or what a page's script block holds).
/// `url` is where it came from, if anywhere, for resolving relative links and as the recipe's URL.
///
/// Fails with [`ReadError::Json`] when it isn't JSON and [`ReadError::NoRecipe`] when it holds no Recipe.
pub fn read_json(json: &str, url: Option<&Url>) -> Result<Vec<Record>, ReadError> {
    let doc: Value = serde_json::from_str(json).map_err(ReadError::Json)?;
    let refs = Refs::of(&doc);
    let mut recipes = Vec::new();
    find_recipes(&doc, 0, &mut recipes);
    if recipes.is_empty() {
        return Err(ReadError::NoRecipe {
            unreadable_blocks: 0,
        });
    }
    let found = recipes.into_iter().map(|r| (r.clone(), &refs)).collect();
    let context = Context {
        base: url,
        page_link: url,
        page_title: None,
    };
    records(found, &context)
}

/// What the page around a recipe says.
struct Context<'a> {
    base: Option<&'a Url>,
    page_link: Option<&'a Url>,
    page_title: Option<&'a str>,
}

/// Objects in a JSON-LD document by `@id`, so `{"@id": "#primaryimage"}` can be looked up.
#[derive(Default)]
struct Refs<'a>(HashMap<&'a str, &'a Value>);

impl<'a> Refs<'a> {
    fn of(doc: &'a Value) -> Refs<'a> {
        let mut refs = HashMap::new();
        let mut stack = vec![(doc, 0)];
        while let Some((value, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                continue;
            }
            match value {
                Value::Array(items) => stack.extend(items.iter().map(|v| (v, depth + 1))),
                Value::Object(map) => {
                    if let Some(Value::String(id)) = map.get("@id")
                        && map.len() > 1
                    {
                        refs.entry(id.as_str()).or_insert(value);
                    }
                    stack.extend(map.values().map(|v| (v, depth + 1)));
                }
                _ => {}
            }
        }
        Refs(refs)
    }

    /// The object a bare reference points at, or the value itself.
    fn resolve<'v>(&self, value: &'v Value) -> &'v Value
    where
        'a: 'v,
    {
        match value {
            Value::Object(map) if map.len() == 1 => map
                .get("@id")
                .and_then(Value::as_str)
                .and_then(|id| self.0.get(id).copied())
                .unwrap_or(value),
            _ => value,
        }
    }
}

/// Parses one script block, tolerating what pages get wrong: an HTML comment or CDATA wrapper, and raw line
/// breaks or tabs inside strings.
fn parse_block(block: &str) -> Option<Value> {
    let mut text = block.trim();
    for (open, close) in [
        ("<!--", "-->"),
        ("//<![CDATA[", "//]]>"),
        ("<![CDATA[", "]]>"),
    ] {
        if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
            text = inner.trim();
        }
    }
    serde_json::from_str(text).ok().or_else(|| {
        // Outside strings a control character is whitespace anyway, so a space is safe everywhere.
        let cleaned: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        serde_json::from_str(&cleaned).ok()
    })
}

fn is_recipe(map: &Map<String, Value>) -> bool {
    let is = |t: &Value| {
        t.as_str()
            .is_some_and(|t| last_segment(t).eq_ignore_ascii_case("Recipe"))
    };
    match map.get("@type") {
        Some(Value::Array(types)) => types.iter().any(is),
        Some(t) => is(t),
        None => false,
    }
}

/// Collects Recipe objects in document order, not looking inside a recipe for more.
fn find_recipes<'a>(value: &'a Value, depth: usize, out: &mut Vec<&'a Value>) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        Value::Array(items) => items.iter().for_each(|v| find_recipes(v, depth + 1, out)),
        Value::Object(map) if is_recipe(map) => out.push(value),
        Value::Object(map) => map.values().for_each(|v| find_recipes(v, depth + 1, out)),
        _ => {}
    }
}

/// Turns each recipe found into a record, dropping repeats (a recipe with the URL and title of one before it, as
/// when a page carries the same JSON-LD twice) and those with no name when no title stands in.
fn records(found: Vec<(Value, &Refs)>, context: &Context) -> Result<Vec<Record>, ReadError> {
    let mut out: Vec<Record> = Vec::new();
    let mut seen_ids: HashMap<String, usize> = HashMap::new();
    for (raw, refs) in found {
        let Some(mut record) = record(raw, refs, context) else {
            continue;
        };
        if out
            .iter()
            .any(|r| r.url == record.url && r.title == record.title)
        {
            continue;
        }
        let n = seen_ids.entry(record.external_id.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            record.external_id = format!("{}#{n}", record.external_id);
        }
        out.push(record);
    }
    if out.is_empty() {
        return Err(ReadError::Untitled);
    }
    Ok(out)
}

fn record(raw: Value, refs: &Refs, context: &Context) -> Option<Record> {
    let r = raw.as_object()?;
    let field = |name: &str| r.get(name).map(|v| refs.resolve(v));

    let title = field("name")
        .or_else(|| field("headline"))
        .and_then(first_text)
        .and_then(|t| line(&t))
        .or_else(|| context.page_title.map(str::to_owned))?;
    let description = field("description")
        .and_then(first_text)
        .map(|t| paragraphs(&t))
        .filter(|t| !t.is_empty());

    let url = ["url", "mainEntityOfPage"]
        .iter()
        .filter_map(|name| field(name).and_then(|v| link(v, refs)))
        .find_map(|href| join(context.base, &href))
        .or_else(|| context.page_link.cloned())
        .map(|mut url| {
            url.set_fragment(None);
            url.to_string()
        });
    let external_id = url.clone().unwrap_or_else(|| {
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(raw.to_string().as_bytes()))
        )
    });

    let yield_values: Vec<String> = field("recipeYield")
        .or_else(|| field("yield"))
        .map(|v| texts(v, refs))
        .unwrap_or_default()
        .iter()
        .filter_map(|t| line(t))
        .collect();
    let mut yields: Vec<f64> = Vec::new();
    for servings in yield_values.iter().filter_map(|t| servings(t)) {
        if !yields.contains(&servings) {
            yields.push(servings);
        }
    }
    let yield_text = yield_values
        .iter()
        .find(|t| t.chars().any(char::is_alphabetic))
        .cloned();

    let ingredients = field("recipeIngredient")
        .or_else(|| field("ingredients"))
        .map(|v| texts(v, refs))
        .unwrap_or_default()
        .iter()
        .flat_map(|t| lines(t))
        .enumerate()
        .map(|(i, name)| Ingredient {
            key: (i + 1).to_string(),
            name,
        })
        .collect();

    let mut steps = Vec::new();
    if let Some(v) = field("recipeInstructions") {
        let mut reader = StepReader {
            refs,
            base: context.base,
            steps: &mut steps,
        };
        reader.read(v, None, 0);
    }

    let images = field("image")
        .map(|v| images(v, refs, context.base))
        .unwrap_or_default();
    let time = |name: &str| field(name).and_then(first_text).and_then(|t| duration(&t));
    let meta = Meta {
        prep_time: time("prepTime"),
        cook_time: time("cookTime"),
        total_time: time("totalTime"),
        yield_text,
        author: field("author").and_then(|v| author(v, refs)),
        cuisines: field("recipeCuisine")
            .map(|v| list(v, refs))
            .unwrap_or_default(),
        categories: field("recipeCategory")
            .map(|v| list(v, refs))
            .unwrap_or_default(),
        tags: field("keywords").map(|v| list(v, refs)).unwrap_or_default(),
        images: images.clone(),
    };

    Some(Record {
        external_id,
        url,
        title,
        description,
        meta,
        yields,
        ingredients,
        steps,
        hero_image: images.into_iter().next(),
        raw,
    })
}

/// Reads `recipeInstructions` in any of its shapes: one text, a list of texts, `HowToStep`s, `HowToSection`s of
/// steps, or an `ItemList`.
struct StepReader<'a> {
    refs: &'a Refs<'a>,
    base: Option<&'a Url>,
    steps: &'a mut Vec<Step>,
}

impl StepReader<'_> {
    fn read(&mut self, value: &Value, section: Option<&str>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let value = self.refs.resolve(value);
        match value {
            Value::String(text) => {
                for text in lines(text) {
                    self.push(strip_numbering(&text).to_owned(), section, Vec::new());
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.read(item, section, depth + 1);
                }
            }
            Value::Object(map) => {
                let kind = map.get("@type").and_then(Value::as_str).map(last_segment);
                let children = map.get("itemListElement");
                match (kind, children) {
                    (Some("HowToSection"), Some(children)) => {
                        let name = map.get("name").and_then(first_text).and_then(|t| line(&t));
                        self.read(children, name.as_deref().or(section), depth + 1);
                    }
                    (Some("ItemList"), Some(children)) => self.read(children, section, depth + 1),
                    _ => {
                        let text = map
                            .get("text")
                            .and_then(first_text)
                            .and_then(|t| line(&t))
                            .or_else(|| children.and_then(|c| self.directions(c, depth + 1)))
                            .or_else(|| {
                                map.get("name").and_then(first_text).and_then(|t| line(&t))
                            });
                        if let Some(text) = text {
                            let images = map
                                .get("image")
                                .map(|v| images(v, self.refs, self.base))
                                .unwrap_or_default();
                            self.push(text, section, images);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// A step written as a list of `HowToDirection`s and `HowToTip`s, joined into one instruction.
    fn directions(&self, value: &Value, depth: usize) -> Option<String> {
        let mut parts = Vec::new();
        let mut stack = vec![(value, depth)];
        while let Some((value, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                continue;
            }
            match self.refs.resolve(value) {
                Value::Array(items) => stack.extend(items.iter().rev().map(|v| (v, depth + 1))),
                Value::Object(map) => {
                    if let Some(text) = map.get("text").and_then(first_text).and_then(|t| line(&t))
                    {
                        parts.push(text);
                    }
                }
                Value::String(text) => parts.extend(line(text)),
                _ => {}
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn push(&mut self, text: String, section: Option<&str>, images: Vec<String>) {
        if text.is_empty() {
            return;
        }
        self.steps.push(Step {
            index: self.steps.len() as i32 + 1,
            text,
            section: section.map(str::to_owned),
            images,
        });
    }
}

/// A string, a number, or the first of a list of them.
fn first_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(items) => items.iter().find_map(first_text),
        Value::Object(map) => map.get("@value").and_then(first_text),
        _ => None,
    }
}

/// Every string and number in a value or a list of them, references followed.
fn texts(value: &Value, refs: &Refs) -> Vec<String> {
    match refs.resolve(value) {
        Value::Array(items) => items.iter().flat_map(|v| texts(v, refs)).collect(),
        Value::Object(map) => map
            .get("name")
            .or_else(|| map.get("text"))
            .and_then(first_text)
            .into_iter()
            .collect(),
        value => first_text(value).into_iter().collect(),
    }
}

/// A comma-separated text or a list of names, as tidy distinct entries: "Dinner, Pasta" → ["Dinner", "Pasta"].
fn list(value: &Value, refs: &Refs) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for text in texts(value, refs) {
        for entry in text.split(',').filter_map(line) {
            if !out.iter().any(|e| e.eq_ignore_ascii_case(&entry)) {
                out.push(entry);
            }
        }
    }
    out
}

/// The name of an author given as text, a Person, or a list of them (the first).
fn author(value: &Value, refs: &Refs) -> Option<String> {
    texts(value, refs).iter().find_map(|t| line(t))
}

/// A URL given as text, an object with `@id` or `url`, or the first of a list.
fn link(value: &Value, refs: &Refs) -> Option<String> {
    match refs.resolve(value) {
        Value::String(s) => Some(s.trim().to_owned()),
        Value::Array(items) => items.iter().find_map(|v| link(v, refs)),
        Value::Object(map) => ["url", "@id", "contentUrl"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_str))
            .map(|s| s.trim().to_owned()),
        _ => None,
    }
}

/// Image URLs from text, `ImageObject`s or a list of either, made absolute; anything not http(s) is left out.
fn images(value: &Value, refs: &Refs, base: Option<&Url>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![(value, 0)];
    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            continue;
        }
        let href = match refs.resolve(value) {
            Value::String(s) => Some(s.as_str()),
            Value::Array(items) => {
                stack.extend(items.iter().rev().map(|v| (v, depth + 1)));
                None
            }
            Value::Object(map) => map
                .get("contentUrl")
                .or_else(|| map.get("url"))
                .and_then(Value::as_str),
            _ => None,
        };
        if let Some(url) = href.and_then(|href| join(base, href.trim())) {
            let url = url.to_string();
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

/// `href` as an absolute http(s) URL, resolved against `base` when it is relative.
fn join(base: Option<&Url>, href: &str) -> Option<Url> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    let url = match base {
        Some(base) => base.join(href).ok()?,
        None => Url::parse(href).ok()?,
    };
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// Text from a value that may hold HTML ("<b>1 cup</b> rice &amp; beans"): tags dropped, entities decoded, a line
/// break at each block element.
fn plain(text: &str) -> String {
    if text.contains(['<', '&']) {
        Html::fragment(text).all_text()
    } else {
        text.to_owned()
    }
}

/// The text on one line with its spaces tidied, or `None` when nothing is left.
fn line(text: &str) -> Option<String> {
    let tidy = plain(text).split_whitespace().collect::<Vec<_>>().join(" ");
    (!tidy.is_empty()).then_some(tidy)
}

/// The text's non-empty lines, each tidied.
fn lines(text: &str) -> Vec<String> {
    plain(text)
        .lines()
        .filter_map(|l| {
            let tidy = l.split_whitespace().collect::<Vec<_>>().join(" ");
            (!tidy.is_empty()).then_some(tidy)
        })
        .collect()
}

/// The text's lines as paragraphs separated by a blank line.
fn paragraphs(text: &str) -> String {
    lines(text).join("\n\n")
}

/// "1. Preheat the oven" → "Preheat the oven"; also "1)" and "Step 1:". Only for steps given as plain text, where
/// the page numbered them itself.
fn strip_numbering(text: &str) -> &str {
    let rest = text
        .strip_prefix("Step ")
        .or_else(|| text.strip_prefix("STEP "))
        .unwrap_or(text);
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || digits > 3 {
        return text;
    }
    let after = &rest[digits..];
    match after.strip_prefix(['.', ')', ':']) {
        Some(stripped) if stripped.starts_with(' ') => stripped.trim_start(),
        _ => text,
    }
}

/// Words that say a yield counts servings.
const SERVING_WORDS: &[&str] = &[
    "serve", "serves", "serving", "servings", "people", "person", "persons", "portion", "portions",
];

/// The serving count a yield gives: "4" → 4; "4 servings", "Serves 4", "4-6 people", "Serves 4 to 6" → 4 (the
/// lower end of a range). `None` when it counts something else ("Makes 24 cookies", "1 loaf").
fn servings(text: &str) -> Option<f64> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '.'))
        .filter(|w| !w.is_empty())
        .collect();
    let number = words.iter().find_map(|w| {
        let w = w.trim_end_matches('.');
        w.parse::<f64>().ok()
    })?;
    if !(number > 0.0 && number <= MAX_SERVINGS) {
        return None;
    }
    let counts_servings = words
        .iter()
        .all(|w| w.parse::<f64>().is_ok() || *w == "to" || *w == "or")
        || words.iter().any(|w| SERVING_WORDS.contains(w));
    counts_servings.then_some(number)
}

/// An ISO 8601 duration in hours and minutes ("PT1H30M") from one written as ISO 8601 ("PT90M", "P0DT1H30M",
/// "PT0H45M") or in words ("1 hour 30 mins"). `None` for nothing, zero, or a duration in months or years.
fn duration(text: &str) -> Option<String> {
    let text = text.trim();
    let seconds = iso_seconds(text).or_else(|| worded_seconds(text))?;
    let minutes = (seconds / 60.0).round() as u64;
    if minutes == 0 || !seconds.is_finite() {
        return None;
    }
    let (hours, minutes) = (minutes / 60, minutes % 60);
    Some(match (hours, minutes) {
        (0, m) => format!("PT{m}M"),
        (h, 0) => format!("PT{h}H"),
        (h, m) => format!("PT{h}H{m}M"),
    })
}

fn iso_seconds(text: &str) -> Option<f64> {
    let upper = text.to_ascii_uppercase();
    let rest = upper.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) => (date, time),
        None => (rest, ""),
    };
    let mut seconds = 0.0;
    let mut any = false;
    for (part, units) in [
        (date, &[('W', 604_800.0), ('D', 86_400.0)][..]),
        (time, &[('H', 3600.0), ('M', 60.0), ('S', 1.0)][..]),
    ] {
        let mut number = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() || c == '.' || c == ',' {
                number.push(if c == ',' { '.' } else { c });
                continue;
            }
            let (_, size) = units.iter().find(|(unit, _)| *unit == c)?;
            seconds += number.parse::<f64>().ok()? * size;
            number.clear();
            any = true;
        }
        if !number.is_empty() {
            return None;
        }
    }
    any.then_some(seconds)
}

fn worded_seconds(text: &str) -> Option<f64> {
    // Runs of digits and of letters, so "1h20m" reads as 1, h, 20, m just as "1 hr 20 min" does.
    let lower = text.to_lowercase();
    let mut tokens: Vec<String> = Vec::new();
    let mut last_kind = None;
    for c in lower.chars() {
        let kind = if c.is_ascii_digit() || c == '.' {
            Some(true)
        } else if c.is_alphabetic() {
            Some(false)
        } else {
            None
        };
        match (kind, last_kind, tokens.last_mut()) {
            (Some(k), Some(last), Some(token)) if k == last => token.push(c),
            (Some(_), _, _) => tokens.push(c.to_string()),
            (None, _, _) => {}
        }
        last_kind = kind;
    }

    let mut seconds = 0.0;
    let mut number: Option<f64> = None;
    let mut any = false;
    for token in tokens.iter().filter(|t| *t != "and") {
        if token.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
            if number.is_some() {
                return None;
            }
            number = Some(token.parse().ok()?);
            continue;
        }
        let size = match token.as_str() {
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "d" | "day" | "days" => 86_400.0,
            _ => return None,
        };
        seconds += number.take()? * size;
        any = true;
    }
    (any && number.is_none()).then_some(seconds)
}

#[cfg(test)]
mod tests;
