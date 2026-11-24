# Backlog

The work still to do, written so one agent session (or one person) can take a task and finish it without
other context. [status.md](status.md) says where things stand and why. How to take a task, claim it, branch, check
and open the merge request is in [AGENTS.md](../AGENTS.md) under "Taking a task".

Each task gives:

- **Touches:** the files or areas it changes. Two tasks that touch the same area shouldn't run at the same time.
- **Done when:** the check that proves it. Every check is automated; "looks right" is not a check.
- **Needs:** tasks to finish first, or a decision or action only Paul, the maintainer, can take (**Paul**).

Take tasks from **Ready** in the order listed, among those whose "Needs" are met: the order puts first what most
other tasks wait on. A task with an open merge request (a draft counts) is taken.
The merge request that finishes a task deletes it here and adds a line under "Where things stand" or "Decisions" in
status.md if it changed either.

## Ready

### T10 · Release images

- On a `v*` tag, a release job in `.gitlab-ci.yml` builds the Docker image, pushes it to the GitLab project's
  container registry, and creates a GitLab release from `CHANGELOG.md` (start one, in Keep a Changelog form, with
  what exists today as 0.1.0).
- **Touches:** `.gitlab-ci.yml`, `CHANGELOG.md`, `docs/deploy.md` (pulling the image instead of building it).
- **Needs:** **Paul** to push the first tag, and the container registry turned on for the project.
- **Done when:** GitLab's CI lint accepts the file, and a test tag (`v0.1.0-rc.1`, pushed by Paul) publishes the
  image to the registry and creates the release; the test release and image are deleted afterwards. Syntax checks
  alone don't prove the release path.

### T35 · Compile-time checked SQL in `mise-api`

Today every query is a string that sqlx checks only when it runs, so a typo or a query the schema has moved away
from fails only in the test that happens to run it.

- Every query in `crates/mise-api/src/` becomes a checked macro (`query!`, `query_as!`, `query_scalar!`) against
  `#[derive(sqlx::FromRow)]` row structs in place of tuple types. A query that must be built at run time (a dynamic
  `ORDER BY`, a table named in a test) keeps the unchecked form with a comment saying why.
- Commit the offline metadata (`.sqlx/`, from `cargo sqlx prepare --workspace`) so builds need no database:
  `SQLX_OFFLINE=true` in the Docker build. `./check`, `.gitlab-ci.yml` and the mirror's `ci.yml` run `cargo sqlx
  prepare --workspace --check`, so a query that no longer matches the migrations, or stale metadata, fails there.
- Move each handler's SQL into a module for its domain (recipes, plans, shopping, accounts) beside the handlers, so
  `recipes.rs` (1,122 lines) splits along what it does. Behaviour stays the same: no route, status or body changes.
- Say in docs/development.md how to refresh `.sqlx/` after changing a query or a migration.
- **Touches:** every file in `crates/mise-api/src/`, `.sqlx/`, `check`, `.gitlab-ci.yml`, `.github/workflows/ci.yml`,
  `Dockerfile`, `docs/development.md`, `docs/architecture.md`.
- **Needs:** nothing else in flight in `crates/mise-api/src/`: it runs alone.
- **Done when:** `cargo sqlx prepare --workspace --check` passes in CI and `./check`; a test-only edit that renames a
  column in one query (not committed) fails `cargo build` with no database; `grep -rn 'sqlx::query(' crates/mise-api/src`
  finds only the commented exceptions; the API tests, `tests/responses.rs` and the generated `schema.d.ts` are
  unchanged.

### T36 · Brands as data

The brands matching strips from ingredient names (`BRANDS` in `mise-core/src/food_names.rs`, 43 of them) are data
kept in code, so adding one means a release. A name marked ® or ™ is already caught without the list.

- Keep brands in the database (a new migration, seeded with today's 43), and let curation add and remove them:
  `{"op": "brand", "name": "Old Bay"}` and `{"op": "unbrand", ...}` in the food curation file, as `mise food apply`
  reads it (docs/curation.md). Food matching loads them once a run and passes them to `strip_brand`, which takes the
  list as an argument; `mise-core` keeps the rules, not the words.
- **Touches:** `crates/mise-core/src/food_names.rs`, `crates/mise-ingest/src/foods.rs` and `curate.rs`, a new
  migration, `docs/curation.md`.
- **Needs:** nothing.
- **Done when:** `BRANDS` is gone from `mise-core`; a test adds a brand through a curation file and a line naming it
  unmarked ("Smoky Hollow Paprika") then matches the unbranded food ("Paprika"), and removing it undoes that; the
  pipeline snapshots (`MISE_UPDATE_SNAPSHOTS` not set) are unchanged.

### T37 · Near-duplicate foods in the review queue

Matching makes a new food for any name it doesn't recognise, and the review queue sees only the matches it was
unsure of. Two foods that are one ("red onion", "red onions, diced") or a confident wrong match pass unseen.

- After matching, pair the foods matching created (not ones a person made or curated) whose names are near: the
  same after plural folding and note stripping, or above a similarity threshold (trigram, as `pg_trgm` scores it).
  Raise each pair as a `food_duplicate` review item with the lines each covers, best first; its answers are `merge`
  (into the one with more lines, or the other), `keep` (two foods), or `set_parent`, stored as the curation commands
  store them, so a rebuild keeps the answer. A pair answered `keep` isn't raised again.
- Report foods per 100 ingredient lines in the import stats and the pipeline snapshot, so a jump shows in review.
- **Touches:** `crates/mise-ingest/src/foods.rs`, `crates/mise-api/src/review.rs`, `web/src/pages/ReviewPage.tsx`,
  `fixtures/demo-bundle/snapshots/`, `docs/curation.md`.
- **Needs:** T38 not in flight (both change the review queue).
- **Done when:** a test with lines "2 red onions" and "1 red onion, diced" in different versions raises one
  `food_duplicate` item, answering `merge` leaves one food covering both and survives `match-foods --reset`, and
  `keep` stops it coming back; the web queue lists and answers it (a browser journey); the pipeline snapshot shows
  foods per 100 lines.

### T38 · Food swaps in the review queue

Grouping raises `food_swap` items (one food replaced by another between a dish's versions: green beans for broccoli),
but nothing records an answer, so the review page doesn't list them (status.md, Known issues).

- List `food_swap` items in the API's queue and the web page, with their dishes and versions. Answers: `alias` (two
  names for one food: a merge), `substitute` (a `food_relations` row of kind `substitute`, both ways), or `keep`
  (unrelated), stored so a rebuild keeps them and a kept pair isn't raised again.
- **Touches:** `crates/mise-api/src/review.rs`, `crates/mise-ingest/src/options.rs`, `web/src/pages/ReviewPage.tsx`,
  `docs/curation.md`.
- **Needs:** T37 not in flight.
- **Done when:** API tests answer a swap each way and see the relation (or the merge) and that a kept pair isn't
  raised again; a browser journey answers one from the review page; status.md's known issue about unlisted swaps is
  gone.

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **Deploy gate:** a CI job that starts the Compose stack on an empty volume, upgrades a database from the previous
  release, and backs up and restores. Needs T10 (a previous release to upgrade from).
- **Presentation:** README screenshots (taken by Playwright from the demo bundle), a hosted demo (**Paul**: where),
  a short demo recording.
- **Scaled counts:** round up to halves (today) or to whole items.

## Only on Paul's machine

These need the private seed, which is not in the repo.

- **`mise gate seed`:** rebuild the private seed and check the invariants on every recipe at 1 to 6 servings, with a
  snapshot of rendered amounts kept in `seed/` and shown as a diff to approve. Include Phase 1's gate in its
  repeatable form: the rebuild gives the same dishes and groupings (counts, and a fingerprint of which versions share
  a dish), with no open review items outside an allowlist. Replace the scaling report's
  "matches the source" score, which counts every round-up as a miss.
- **Deploy the stack on the seed:** rebuild the image, run the full Compose stack, copy the `seed/` files deploy.md
  names to the server (the `-seed` versions of curation, dish decisions and preferred versions).
- **HTTPS for the household's install:** proxy mise through the existing Caddy, as deploy.md section 7 says.
- **Dish addresses on the seed:** run `group-dishes` (then `derive-options`) on the seed once T21's migration is in,
  so renamed dishes such as the pot pie take addresses that match their names; then drop that known issue from
  status.md.
- **Off-machine backups** of the database and `seed/`.
- **GitHub mirror settings:** turn on secret scanning (free on public repositories; GitLab's pipeline already runs
  secret detection).

## Later phases

Phase 3, still to build:

- Preferred products and stores per food; per-store aisle order.
- "Make from scratch": swap a proprietary item for its components.
- Nextcloud Tasks (the list as CalDAV to-dos) and Calendar (the plan as events or an iCal feed).
- Days in the plan (meals belong to a week today).

Phase 4, kitchen and imports, was broken into T22 to T33, all done: recipes owned by a household (T22), kitchen
devices and mode (T23, T24), import from a link and from files (T26, T27), writing recipes and the editor (T28,
T29), forks (T30), sharing between households (T31), review queues in the web app (T32) and printable cards (T33).
Its gate, T24's kitchen journey and T25's 20 sample pages, passes.

Break a phase item into Ready tasks, in the shape above, before starting it.
