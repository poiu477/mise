# Backlog

The work still to do, written so one agent session (or one person) can take a task and finish it without
other context. [status.md](status.md) says where things stand and why; the design document holds the long-range
goals. How to take a task, branch, check and open the pull request is in [CLAUDE.md](../CLAUDE.md) under
"Working as an agent".

Each task gives:

- **Touches:** the files or areas it changes. Two tasks that touch the same area shouldn't run at the same time.
- **Done when:** the check that proves it. Every check is automated; "looks right" is not a check.
- **Needs:** tasks to finish first, or a decision or action only Paul can take (**Paul**).

Take tasks from **Ready** in roughly the order listed. When a task is merged, delete it here and add a line under
"Where things stand" or "Decisions" in status.md if it changed either, in the same pull request.

## Ready

### T1 · One local check command

Contributors should run CI's code checks with one command, before pushing.

- Add `./check` (a bash script; it must work in Git Bash on Windows and on Linux). It runs CI's commands with CI's
  flags, in CI's order: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`;
  `cargo test --workspace --locked`; the API step (write the OpenAPI document, run `check-openapi.mjs`, regenerate
  `schema.d.ts` and fail if it differs); then in `web/` format, lint, test and build. `--e2e` adds the browser
  journeys. It stops at the first failure and names it.
- Out of scope, and said so in `./check --help` and development.md: `npm audit`, `cargo deny` and the Docker image
  build stay in CI only (they need the network or take minutes and rarely fail on a code change).
- Add `.githooks/pre-commit` running the fast part (formatting and lint only), enabled with
  `git config core.hooksPath .githooks`; document that in development.md.
- **Touches:** `check`, `.githooks/`, `docs/development.md`, the commands block in `CLAUDE.md`.
- **Done when:** `./check` passes on a clean `main`, and exits non-zero after breaking formatting in one Rust and
  one TypeScript file (try it, then revert). CI itself is unchanged.

### T2 · One test harness for the API tests

`crates/mise-api/tests/auth.rs`, `plan.rs` and `oidc.rs` each create a throwaway database, run migrations, build
the router and sign people in their own way.

- Move that into `crates/mise-api/tests/common/mod.rs`: a `TestApp` that owns the database (dropped afterwards), the
  router, and helpers to set up the first owner, sign in, and send JSON with the session cookie. Keep the rule that
  tests fail, not skip, in CI when the database is missing.
- **Touches:** `crates/mise-api/tests/` only.
- **Done when:** the same tests pass with the same names; no setup code is repeated across the three files.

### T3 · API tests for dishes, versions and photos

Only sign-in, plan and shopping have HTTP-level tests.

- Load `fixtures/demo-bundle` into the test database (through `mise-ingest`, as `web/e2e/serve.mjs` does with
  the CLI) and test: search, a dish page, a version at a published and a scaled serving count, option toggles,
  photo and missing-photo responses, and that signed-out requests get 401.
- **Touches:** `crates/mise-api/tests/` (new files).
- **Needs:** T2.
- **Done when:** the new tests pass locally and in CI.

### T4 · Pipeline snapshot test

A change to grouping, matching, step parsing or rounding should show up as a reviewable diff.

- A test that imports `fixtures/demo-bundle` through every stage (import, match foods, apply curation, group dishes,
  derive options, parse steps), checks the counts, and compares every dish page at 1 to 6 servings with a
  committed snapshot (the `insta` crate, or plain files under `fixtures/demo-bundle/snapshots/`).
- **Touches:** a new test in `crates/mise-ingest` or `crates/mise-cli`, `fixtures/demo-bundle/`.
- **Done when:** the test passes in CI; changing one rounding rule makes it fail with a readable diff (try it, then
  revert).

### T5 · A data-fetching layer for the web app

Pages load data with `useEffect` and `useState`, which is why ESLint's `react-hooks/set-state-in-effect` is a
warning (9 places) rather than an error.

- Move page data loading to TanStack Query (or one shared hook, if that reads simpler; say which and why in the
  pull request). Keep the offline shopping list's own device copy and queue (`web/src/offline.ts`) as it is.
- Turn `react-hooks/set-state-in-effect` to `"error"` in `web/eslint.config.js`.
- **Touches:** `web/src/pages/`, `web/src/components/`, `web/src/api/`, `web/package.json`.
- **Done when:** `npm run lint` reports no warnings with the rule as an error, and the browser journeys pass.

### T6 · Security headers and a written security stance

- Add response headers from the server: a Content-Security-Policy that the app and its service worker work under,
  `frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, a referrer policy.
- Write `SECURITY.md` (how to report a problem, what is in scope) and a short section in `docs/architecture.md` on
  the CSRF stance (SameSite=Lax cookies plus JSON-only bodies) and login throttling.
- **Touches:** the router setup in `crates/mise-api/src/lib.rs`, `SECURITY.md`, `docs/architecture.md`.
- **Done when:** an API test checks each header on a page and an API response, and the browser journeys pass (they
  would catch a policy that breaks the app).

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

- On a `v*` tag, build the Docker image and push it to GitHub's container registry, and draft a GitHub release from
  `CHANGELOG.md` (start one, in Keep a Changelog form, with what exists today as 0.1.0).
- **Touches:** `.github/workflows/release.yml`, `CHANGELOG.md`, `docs/deploy.md` (pulling the image instead of
  building it).
- **Needs:** **Paul** to push the first tag.
- **Done when:** the workflow passes `actionlint` with its steps pinned to SHAs like `ci.yml`, and a test tag
  (`v0.1.0-rc.1`, pushed by Paul) publishes the image to GitHub's registry and drafts the release; the test release
  and image are deleted afterwards. Syntax checks alone don't prove the release path.

### T11 · Merge shopping lines across related foods and units

Onion and Yellow Onion are two lines on the list today, and "1 TBSP + 2 cloves" of garlic stays joined.

- Add lines up at the parent food when the plan uses several kinds of one food, and across units when a food has a
  density (a new table of per-food densities, through a new migration).
- **Touches:** `crates/mise-core/src/shopping.rs`, `crates/mise-api/src/shopping.rs`, a new migration, the demo
  bundle (add a case that exercises it).
- **Done when:** unit tests cover both merges; a browser journey shows one onion line for the demo week.

### T12 · Adding to the list offline must not add twice

An item added offline can appear twice if the server saves it but its answer is lost: adding has no idempotency
key, so the device sends it again (status.md, known issues).

- Give each queued addition a client-made id (a UUID made when it is queued, kept with it in the device copy) and
  have the server treat a repeated id from the same household as the same addition.
- **Touches:** `web/src/offline.ts`, the add-item route in `crates/mise-api/src/shopping.rs`, a new migration,
  `web/src/api/schema.d.ts` (regenerated).
- **Done when:** an API test sends the same addition twice and gets one item; a unit test in `web/` shows a retried
  queue sends the same id; the known issue is removed from status.md.

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **API contract:** `/api/v1` and one error shape, before anything outside the web app depends on the API.
- **Database code:** `#[derive(sqlx::FromRow)]` structs, compile-time checked queries with `.sqlx` committed, SQL
  moved out of handlers into per-domain modules. It touches every file in `crates/mise-api/src/`, so it runs
  alone, with nothing else in flight there. Decide whether to do it before or after T7.
- **Deploy gate:** a CI job that starts the Compose stack on an empty volume, upgrades a database from the previous
  release, and backs up and restores. Needs T10 (a previous release to upgrade from).
- **Presentation:** README screenshots (taken by Playwright from the demo bundle), a hosted demo (**Paul**: where),
  a short demo recording.
- **Scaled counts:** round up to halves (today) or to whole items.

## Only on Paul's machine

These need the private seed, which is not in the repo. See `CLAUDE.local.md` there.

- **`mise gate seed`:** rebuild the private seed and check the invariants on every recipe at 1 to 6 servings, with a
  snapshot of rendered amounts kept in `seed/` and shown as a diff to approve. Replace the scaling report's
  "matches the source" score, which counts every round-up as a miss.
- **Deploy the stack on the seed:** rebuild the image, run the full Compose stack, copy the `seed/` files deploy.md
  names to the server (the `-seed` versions of curation, dish decisions and preferred versions).
- **HTTPS** through the reverse proxy, which phones need to open the list offline.
- **Off-machine backups** of the database and `seed/`.
- **GitHub settings:** turn on secret scanning and push protection (free on public repositories).

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
