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

### T9 · Observability

- An option for JSON logs, a request id on every log line (and in the response), a readiness check that the
  database answers (next to the existing health check), and basic metrics (request counts and durations by route).
- **Touches:** `crates/mise-api/src/main.rs`, `lib.rs`, `docs/deploy.md`.
- **Needs:** nothing else in flight in `crates/mise-api/src/lib.rs`.
- **Done when:** API tests cover the readiness check and the request id header; deploy.md says how to turn JSON logs
  on.

### T10 · Release images

- On a `v*` tag, a release job in `.gitlab-ci.yml` builds the Docker image, pushes it to the GitLab project's
  container registry, and creates a GitLab release from `CHANGELOG.md` (start one, in Keep a Changelog form, with
  what exists today as 0.1.0).
- **Touches:** `.gitlab-ci.yml`, `CHANGELOG.md`, `docs/deploy.md` (pulling the image instead of building it).
- **Needs:** **Paul** to push the first tag, and the container registry turned on for the project.
- **Done when:** GitLab's CI lint accepts the file, and a test tag (`v0.1.0-rc.1`, pushed by Paul) publishes the
  image to the registry and creates the release; the test release and image are deleted afterwards. Syntax checks
  alone don't prove the release path.

### T16 · Rust doc comments: `mise-api`, then the lint for the whole workspace

- As `mise-core`, `mise-db` and `mise-ingest` already do, for `crates/mise-api` (213 items). In this crate `///` on a
  handler or a request or response type also becomes its description in the OpenAPI document, so regenerate
  `web/src/api/schema.d.ts` and keep `check-openapi.mjs` passing; write those comments for an API reader.
- Then move the lint to the workspace: `[workspace.lints.rust] missing_docs = "warn"` in `Cargo.toml` and
  `[lints] workspace = true` in every crate's `Cargo.toml`, removing the per-crate attributes. `mise-cli` is a binary
  and has no public items, so it's covered without work.
- **Touches:** `crates/mise-api/src/` (comments only), `web/src/api/schema.d.ts` (regenerated), every `Cargo.toml`.
- **Needs:** not while the database-code rework is in flight: it moves most of this crate.
- **Done when:** clippy passes workspace-wide with the lint in `Cargo.toml`; a missing doc comment in any crate fails
  it.



### T27 · Import from files

- Export files from other apps through the same records and shared stages as T26: schema.org JSON, Nextcloud
  Cookbook (a folder of `recipe.json` files), Mealie, Tandoor and Paprika (`.paprikarecipes`). `mise import file
  <path>` and an upload form in the web app; each format's fixture is written for mise.
- **Touches:** `crates/mise-ingest/src/` (a module per format), `crates/mise-cli/src/`, `crates/mise-api/src/`,
  `web/src/`, `fixtures/`.
- **Needs:** T22, T25.
- **Done when:** a test per format imports its fixture to the expected records; a browser journey uploads one file
  and opens the dish.

### T34 · Dates and times without chrono

chrono is unmaintained (RUSTSEC-2026-0543, accepted in `deny.toml` until this and openidconnect allow otherwise).

- Move mise's own code from chrono to `time` (already a dependency of `mise-api`) or `jiff`, whichever reads more
  plainly in the week and plan code: `mise-ingest`, `mise-api`, `mise-cli`, and sqlx's and utoipa's
  `chrono` features for their `time` ones. Weeks are named by their Monday and plans are dates, so most of it is
  `NaiveDate`.
- chrono stays in `Cargo.lock` through `oauth2` and `openidconnect` until they drop it; say so in the merge request,
  and keep the `deny.toml` entry until `cargo tree -i chrono` shows only them.
- **Touches:** every crate's dates (`plan.rs`, `shopping.rs`, `week` handling, the gate report), every `Cargo.toml`.
- **Needs:** nothing else in flight in `crates/mise-api/src/plan.rs` or `shopping.rs`.
- **Done when:** `cargo tree -i chrono -e normal` lists only `oauth2` and `openidconnect`; the API document's date and
  time formats are unchanged (`web/scripts/check-openapi.mjs` and a diff of `web/src/api/schema.d.ts` show it).

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **Database code:** `#[derive(sqlx::FromRow)]` structs, compile-time checked queries with `.sqlx` committed, SQL
  moved out of handlers into per-domain modules. It touches every file in `crates/mise-api/src/`, so it runs
  alone, with nothing else in flight there.
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

Phase 4, kitchen and imports, is broken into T22 to T33 under Ready: import from files is left (recipes owned by a
household, T22, kitchen mode, T24, import from a link, T26, writing recipes and the editor, T28 and T29, forks, T30,
sharing between households, T31, review queues in the web app, T32, and printable cards, T33, are done). Its gate, T24's
kitchen journey and T25's 20 sample pages, passes.

Break a phase item into Ready tasks, in the shape above, before starting it.
