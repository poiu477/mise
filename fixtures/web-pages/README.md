# Sample recipe pages

Twenty web pages **written for mise**, each shaped like the markup a common kind of recipe site publishes, with the
records `mise_ingest::schema_org::read_page` should make of them. They are the gate for reading schema.org Recipe
data: `crates/mise-ingest/src/schema_org/tests.rs` reads each `<name>.html` as fetched from
`https://fixtures.example/<name>.html` and compares the result with `<name>.json`.

Nothing here is saved or copied from a real site, and the recipes, names and sites are made up (the sites use the
reserved `.example` domain). They're under the repository's MIT licence.

| Page | What it covers |
| --- | --- |
| `blog-plugin-graph` | A blog recipe plugin's JSON-LD inside an SEO plugin's `@graph`: yield as `["4", "4 servings"]`, three image crops, nutrition |
| `blog-plugin-standalone` | Another plugin's standalone Recipe block with named `HowToStep`s; leftover microdata from an old theme, which JSON-LD outranks |
| `graph-references` | `@graph` where the recipe's image and author are `{"@id": …}` references to other nodes |
| `top-level-array` | The script holds a JSON array (breadcrumbs, then the recipe); a numeric yield; the URL from `og:url` |
| `microdata-only` | Classic microdata: `meta` and `time` times, a relative image, the method as one `ol` |
| `microdata-howto-steps` | Microdata `HowToStep` items with their own image; unclosed `<li>` ingredients; "Serves 2"; no canonical link |
| `microdata-nested` | A microdata Recipe inside a `WebPage` item, times in `content` attributes, nested author and nutrition items; the page's JSON-LD has no recipe |
| `sections` | `HowToSection`s ("For the dressing", "For the slaw") |
| `yield-range` | "4-6 servings" |
| `yield-makes` | "Makes 24 cookies", which counts cookies, not servings |
| `no-image` | No image at all, and no canonical link |
| `html-entities` | Entities and tags inside JSON-LD text: `&amp;`, `&#8217;`, `&frac12;`, `<strong>`, `<br>` |
| `instructions-html-string` | The method as one HTML string (`<ol><li>…`) |
| `instructions-plain-lines` | The method as numbered lines in one string; ingredients in the old `ingredients` property as one string |
| `type-list-headline` | `@type: ["Recipe", "NewsArticle"]`, a `headline` but no `name`, two authors, keywords as a list |
| `odd-times` | `PT0H15M`, "1 hr 30 mins", `PT105M`, and a zero `P0DT0H0M` |
| `two-recipes` | Two recipes on one page, in separate blocks |
| `relative-urls` | A `<base>`, a relative canonical link, relative and protocol-relative images, `data:` and `javascript:` images left out |
| `messy-json` | A CDATA wrapper, raw line breaks in strings, a block that isn't valid JSON, and the recipe repeated in a commented block |
| `step-images` | Step images as text and as `ImageObject`, and a step made of a `HowToDirection` and a `HowToTip` |

`no-recipe.html` is the twenty-first page: an article with JSON-LD and microdata but no recipe, which must give the
"no recipe found" error.

Each `<name>.json` is a list of records (one, or two for `two-recipes`), each a line the seed bundle importer can
read (see [docs/seed-bundle.md](../../docs/seed-bundle.md)) minus `raw`, the page's own Recipe object, which the test
checks separately.

When a change to the reader is meant to change what it reads, rewrite the expected files and review the diff:

```bash
MISE_UPDATE_SNAPSHOTS=1 cargo test -p mise-ingest schema_org
git diff fixtures/web-pages/
```
