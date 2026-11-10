# Backlog

The work still to do, written so one agent session (or one person) can take a task and finish it without
other context. [status.md](status.md) says where things stand and why. How to take a task, claim it, branch, check
and open the merge request is in [AGENTS.md](../AGENTS.md) under "Taking a task".

Each task gives:

- **Touches:** the files or areas it changes. Two tasks that touch the same area shouldn't run at the same time.
- **Done when:** the check that proves it. Every check is automated; "looks right" is not a check.
- **Needs:** tasks to finish first, or a decision or action only Paul, the maintainer, can take (**Paul**).

Take tasks from **Ready** in roughly the order listed. A task with an open merge request (a draft counts) is taken.
The merge request that finishes a task deletes it here and adds a line under "Where things stand" or "Decisions" in
status.md if it changed either.

## Ready


### T4 · Pipeline snapshot test

A change to grouping, matching, step parsing or rounding should show up as a reviewable diff.

- A test that imports `fixtures/demo-bundle` through every stage (import, match foods, apply curation, group dishes,
  derive options, parse steps), checks the counts, and compares every dish page at 1 to 6 servings with a
  committed snapshot (the `insta` crate, or plain files under `fixtures/demo-bundle/snapshots/`).
- **Touches:** a new test in `crates/mise-ingest` or `crates/mise-cli`, `fixtures/demo-bundle/`.
- **Done when:** the test passes in CI; changing one rounding rule makes it fail with a readable diff (try it, then
  revert).

### T6 · Security headers and a written security stance

- Add response headers from the server: a Content-Security-Policy that the app and its service worker work under,
  `frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, a referrer policy.
- Write `SECURITY.md` (how to report a problem, what is in scope) and a short section in `docs/architecture.md` on
  the CSRF stance as the code enforces it: SameSite=Lax session cookies, JSON bodies where a route takes one, and
  what protects each state-changing route that takes no body (sign-out, the DELETE routes). Check each in the code;
  if one turns out unprotected, fix it (for example by requiring a custom header) rather than describe a protection
  that isn't there. Include login throttling.
- **Touches:** the router setup in `crates/mise-api/src/lib.rs`, `SECURITY.md`, `docs/architecture.md`.
- **Done when:** an API test checks each header on a page and an API response; an API test shows a cross-site
  style request (no JSON content type, no custom header) to sign-out and to a DELETE route is refused or harmless,
  as the written stance says; the browser journeys pass (they would catch a policy that breaks the app).

### T7 · Document every response each endpoint can return

The OpenAPI annotations list the main responses, but every signed-in route can also return 401 and 412
(`X-Mise-Account`), browser-only actions 403, and some validation 400s are missing.

- Add the missing responses to the `#[utoipa::path]` annotations, then a test that calls each route in the ways
  that produce each documented status and fails on a status the document doesn't list.
- **Touches:** `crates/mise-api/src/` (annotations only), a new test in `crates/mise-api/tests/`,
  `web/src/api/schema.d.ts` (regenerated).
- **Needs:** T2.
- **Done when:** the new test passes; `web/scripts/check-openapi.mjs` still passes.

### T9 · Observability

- An option for JSON logs, a request id on every log line (and in the response), a readiness check that the
  database answers (next to the existing health check), and basic metrics (request counts and durations by route).
- **Touches:** `crates/mise-api/src/main.rs`, `lib.rs`, `docs/deploy.md`.
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

### T11 · Merge shopping lines across related foods and units

Onion and Yellow Onion are two lines on the list today, and "1 TBSP + 2 cloves" of garlic stays joined.

- Add lines up at the parent food when the plan uses several kinds of one food, and across units when the food has
  the data to convert: a density for weight and volume, and a size per item (weight or volume of one clove, one
  onion) for counts. A count with no size per item stays its own line; density alone never turns "2 cloves" into
  tablespoons. Densities and item sizes go in a new table, through a new migration.
- Ticks belong to lines, so define and test what happens to them: a merged line is ticked when every line it
  replaced was ticked (a mix shows as not ticked); ticking it ticks them all; ticks queued offline for a line that
  has since merged or split apply to whatever lines now cover that food; a plan change that splits a merged line
  again keeps each part's tick.
- **Touches:** `crates/mise-core/src/shopping.rs`, `crates/mise-api/src/shopping.rs`, a new migration, the demo
  bundle (add a case that exercises it).
- **Done when:** unit tests cover both merges, the count that can't convert, and each tick rule above; a browser
  journey shows one onion line for the demo week and keeps its tick across a reload.


### T16 · Rust doc comments: `mise-api`, then the lint for the whole workspace

- As `mise-core`, `mise-db` and `mise-ingest` already do, for `crates/mise-api` (213 items). In this crate `///` on a
  handler or a request or response type also becomes its description in the OpenAPI document, so regenerate
  `web/src/api/schema.d.ts` and keep `check-openapi.mjs` passing; write those comments for an API reader.
- Then move the lint to the workspace: `[workspace.lints.rust] missing_docs = "warn"` in `Cargo.toml` and
  `[lints] workspace = true` in every crate's `Cargo.toml`, removing the per-crate attributes. `mise-cli` is a binary
  and has no public items, so it's covered without work.
- **Touches:** `crates/mise-api/src/` (comments only), `web/src/api/schema.d.ts` (regenerated), every `Cargo.toml`.
- **Needs:** T7 (it rewrites the same annotations). Not while the database-code rework is in flight: it
  moves most of this crate.
- **Done when:** clippy passes workspace-wide with the lint in `Cargo.toml`; a missing doc comment in any crate fails
  it.

### T17 · Published Rust documentation

- A CI job that runs `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS="-D warnings"` on every merge request
  (broken intra-doc links and bad doc markup fail it), and on `main` publishes the result to GitLab Pages (a `pages`
  job). Link it from the README.
- Run the same `cargo doc` check from `./check` (T1), so a local run catches what this job would.
- **Touches:** `.gitlab-ci.yml`, `.github/workflows/ci.yml` (the same check on the mirror, without publishing),
  `check`, `README.md`.
- **Needs:** **Paul** to make sure GitLab Pages is enabled on the server and the
  project.
- **Done when:** the job passes, fails on a deliberately broken intra-doc link (try it, then revert), and the Pages
  URL serves `mise_core`'s documentation.

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **API contract:** `/api/v1` and one error shape, before anything outside the web app depends on the API.
- **Database code:** `#[derive(sqlx::FromRow)]` structs, compile-time checked queries with `.sqlx` committed, SQL
  moved out of handlers into per-domain modules. It touches every file in `crates/mise-api/src/`, so it runs
  alone, with nothing else in flight there. Order with T7 is settled: either can go first (T7 changes the OpenAPI
  annotations and adds tests, this changes queries and their modules), but never both at once.
- **Deploy gate:** a CI job that starts the Compose stack on an empty volume, upgrades a database from the previous
  release, and backs up and restores. Needs T10 (a previous release to upgrade from).
- **Presentation:** README screenshots (taken by Playwright from the demo bundle), a hosted demo (**Paul**: where),
  a short demo recording.
- **Scaled counts:** round up to halves (today) or to whole items.

## Only on Paul's machine

These need the private seed, which is not in the repo.

- **`mise gate seed`:** rebuild the private seed and check the invariants on every recipe at 1 to 6 servings, with a
  snapshot of rendered amounts kept in `seed/` and shown as a diff to approve. Replace the scaling report's
  "matches the source" score, which counts every round-up as a miss.
- **Deploy the stack on the seed:** rebuild the image, run the full Compose stack, copy the `seed/` files deploy.md
  names to the server (the `-seed` versions of curation, dish decisions and preferred versions).
- **HTTPS** through the reverse proxy, which phones need to open the list offline.
- **Off-machine backups** of the database and `seed/`.
- **GitHub mirror settings:** turn on secret scanning (free on public repositories; GitLab's pipeline already runs
  secret detection).

## Later phases

Phase 3, still to build:

- Preferred products and stores per food; per-store aisle order.
- "Make from scratch": swap a proprietary item for its components.
- Nextcloud Tasks (the list as CalDAV to-dos) and Calendar (the plan as events or an iCal feed).
- Days in the plan (meals belong to a week today).

Phase 4, kitchen and imports:

- Kitchen mode for the Echo Show: one step at a time, large type, timers from step times, screen kept awake, a
  device token instead of a sign-in.
- Import from a URL (schema.org Recipe data) and from files (schema.org JSON, Mealie, Tandoor, Paprika, Nextcloud
  Cookbook).
- A manual editor and forks: your own versions of any dish.
- Review queues in the web app (CLI and report only today).
- Printable recipe cards.

Break a phase item into Ready tasks, in the shape above, before starting it.
