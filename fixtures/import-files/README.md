# Sample export files

An export from each recipe app mise imports from, **written for mise**, laid out as the app lays out its export, with
the records `mise_ingest::files` should make of them. They are the tests for import from files:
`crates/mise-ingest/src/files/tests.rs` reads each folder and compares what it gives with `expected/<folder>.json`,
then packs each folder as its app packs the export (a zip, a zip of zips, a zip of gzipped recipes) and checks the
export reads the same; `crates/mise-api/tests/file_imports.rs` imports each packed export through the API.

Nothing here is copied from a real app's library or a real site. The recipes, names and links are made up (the links
use the reserved `.example` domain), the photos are small solid pictures drawn for these tests, and the app-specific
ids are invented. They're under the repository's MIT licence.

| Folder | The app's export, and what it covers |
| --- | --- |
| `schema-org` | One recipe saved as schema.org JSON-LD: an author, a cuisine, keywords, and a photo only linked to (not fetched) |
| `nextcloud-cookbook` | The `Recipes` folder: a recipe folder each, `recipe.json` with Nextcloud's own `id` and its empty `url` and `image`, `full.jpg` with its thumbnails beside it (taken as the photo), steps as text and as `HowToStep`s, a numeric yield |
| `mealie` | `recipes/<slug>/<slug>.json` in snake_case, as the export zip names fields, and `images/original.webp`: lines written as typed (`original_text`), read into parts only, and kept in the note alone; a step title starting a section; times in seconds beside the text |
| `tandoor` | One recipe's folder (packed as `1.zip` inside the export): steps each with their ingredients, headers among them, an ingredient with no amount, plural names, times in minutes, and `image.jpg` |
| `paprika` | Two recipes as JSON (packed gzipped as `.paprikarecipe` files in a `.paprikarecipes` zip): ingredients and directions as text with a blank line and numbering, the photo inline as base64, and a yield that counts jars, not servings |

Each `expected/<folder>.json` lists the recipes the folder gives, in path order: the format, where in the export it
was, where its photo was (a file beside it, or `…#photo_data` inside a Paprika recipe), and the record minus `raw`
(the app's own recipe object, which tests check separately).

When a change to the reader is meant to change what it reads, rewrite the expected files and review the diff:

```bash
MISE_UPDATE_SNAPSHOTS=1 cargo test -p mise-ingest files
git diff fixtures/import-files/
```
