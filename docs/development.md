# Development

Requirements: Rust 1.99.1 (pinned in `rust-toolchain.toml`; rustup installs it on first build), Node 26, Docker,
Python 3 for the private seed scripts. The same Rust builds everywhere: locally, in CI and in the Docker image.

## First setup

```bash
cp .env.example .env
docker compose -f compose.dev.yaml up -d db      # Postgres 16 on localhost:5433, user and password mise
cd web && npm install && cd ..
```

## Databases

| Database | What | Used for |
| --- | --- | --- |
| `mise_seed` | The launch seed: 237 dishes and 308 versions from meals the household received | Day-to-day use and development (`.env` points here) |
| `mise` | The full private archive import, about 9,900 dishes | Evidence: package sizes, pack counts, more dishes to add later |
| `mise_test_<uuid>` | Made and dropped by each API integration test | Tests only |

Switching between them is a matter of `DATABASE_URL` and `MEDIA_ROOT` in `.env`; `seed/env-full-archive` holds the
full archive's pair.

## Rebuilding the seed database

The private inputs live in `seed/` (ignored by git); the photos come from the archive's folder (`ARCHIVE` below).

```bash
docker exec mise-db-1 createdb -U mise mise_seed
cargo build --release -p mise-cli
M=./target/release/mise
ARCHIVE=/path/to/archive        # the private archive's photo folder
$M migrate
$M import-bundle seed/mise-seed
$M match-foods
$M food apply seed/curation-seed.json
$M dish apply seed/dish-decisions-seed.json
$M dish prefer seed/preferred-versions.json
$M group-dishes
$M derive-options
$M parse-steps
$M ingest-media --from "$ARCHIVE"
```

The result should be 237 dishes and 308 versions. `seed/build-seed-bundle.py`, `build-count-units.py` and
`build-store-sections.py` regenerate the bundle and parts of the curation file from the archive; each says at the top
what it reads and writes.

## Running

```bash
cargo run --release -p mise-api           # mise-server on :8080, serving web/dist and the API
cd web && npm run dev                     # Vite on :5173 with /api forwarded to :8080
```

The first visit to an empty database creates the admin account. `mise user set-password <name>` (new password on
standard input) recovers a lost one. The OpenAPI document is at `/api/openapi.json`.

A second server for trying things, without touching the one in daily use:

```bash
MISE_BIND=127.0.0.1:8091 MISE_WEB_DIR=web/dist ./target/debug/mise-server.exe
```

The service worker only registers in production builds (`npm run build`), and the built-in browser pane in the
Claude desktop app doesn't run it, so offline start-up has to be tried in a real browser.

## Tests

`./check` runs CI's build, lint and test steps in CI's order, and stops at the first failure, naming the step:

```bash
./check            # Rust formatting, clippy, tests, the API document check, then web formatting, lint, tests, build
./check --e2e      # the same, then the browser journeys
./check --fast     # formatting and lints only
```

It runs the test steps with `CI` set, as CI does. So the API tests fail when `DATABASE_URL` (from the environment or
`.env`, and passed on to the browser journeys) is missing or unreachable, rather than skipping, and Vitest and
Playwright refuse a focused `.only` test. As in CI, it regenerates `web/src/api/schema.d.ts` and fails if git shows a
change, leaving the new file to commit. It runs `npm ci` only when `web/node_modules` is missing or older than
`web/package-lock.json`. Without `--e2e` it skips the browser journeys. `cargo deny`, `npm audit` and the Docker
image build stay CI-only: `./check` never runs them.

A pre-commit hook runs `./check --fast` on the working tree before each commit. Turn it on once per clone:

```bash
git config core.hooksPath .githooks
```

`git commit --no-verify` skips it for one commit. The steps one by one:

| What | Command | Notes |
| --- | --- | --- |
| Rust formatting | `cargo fmt --all` (CI: `--check`) | rustfmt at its defaults (`rustfmt.toml`) |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` | Zero warnings |
| Rust unit, property and integration tests | `cargo test --workspace` | API tests need `DATABASE_URL` and make a throwaway database each; skipped when it isn't set |
| Rust dependencies | `cargo deny check` | Advisories, licenses, bans and sources (`deny.toml`); CI runs it |
| Web formatting and lints | `cd web && npm run format` and `npm run lint` | Prettier at width 100; ESLint with the React hooks rules |
| Web unit tests | `cd web && npm test` | Vitest; pure modules (`choose.ts`, `offline.ts`, `week.ts`) |
| Types and build | `cd web && npm run build` | Runs the TypeScript check first |
| Browser journeys | `cd web && npm run e2e` | Playwright in Chromium against `mise-server` on a fresh `mise_e2e` database built from the demo recipes (`fixtures/demo-bundle`). Build first: `cargo build -p mise-cli -p mise-api` and `npm run build`; once, `npx playwright install chromium`. Covers first-run setup, scaling, planning (the Phase 3 gate: under 20 interactions), shopping offline including opening with no connection, phone width, an axe accessibility scan and sign-out |
| Scaling report | `mise scaling-report --sample 300 --out reports/scaling.html` | Compares derived amounts with what the source published; read-only |
| Grouping report | `mise gate-report` | A stratified sample of dishes to check grouping by hand |

The fuzz suite in `crates/mise-core/tests/fuzz.rs` takes about 25 seconds in a debug build. The plan for the
remaining repeatable gates (a seed gate) is in [status.md](status.md) and the design document.

## CI

`.github/workflows/ci.yml` runs on every push to `main` and every pull request: the Rust job (formatting, clippy,
tests against a Postgres service, a check that every response in the OpenAPI document has the description OpenAPI 3.1 requires (`web/scripts/check-openapi.mjs`), and a check that `web/src/api/schema.d.ts` matches the server), the web job
(formatting, lint, tests, build, an audit of shipped npm packages), the browser journeys (keeping Playwright's report
and traces when they fail), `cargo deny`, and a build of the Docker image.
Dependabot opens grouped update pull requests weekly. TypeScript majors are held back until every tool that depends on TypeScript accepts them: the weekly "TypeScript major watch" workflow (`npm run typescript:ready` in `web/`) opens an issue when that happens, and deleting the `typescript` ignore rule in `.github/dependabot.yml` lets Dependabot propose the upgrade. `.git-blame-ignore-revs` lists the formatting-only commits;
run `git config blame.ignoreRevsFile .git-blame-ignore-revs` once per clone.

## Changing the API

Add the handler with a `#[utoipa::path]` attribute, register it in `ApiDoc` and the router in `lib.rs`, add an
integration test in `crates/mise-api/tests/`, then regenerate the web types (`cd web && npm run api`) and add the
type alias to `web/src/api/client.ts`.

## Changing the schema

Add `migrations/00NN_<what>.sql` with a first-line comment saying what it adds. The server and `mise migrate` apply
it. An older binary refuses to start against a database that has a migration it doesn't know, so rebuild before
restarting after pulling.
