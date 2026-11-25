# Development

Requirements: Rust 1.100.0 (pinned in `rust-toolchain.toml`; rustup installs it on first build), Node 26, Docker and
bash (Git Bash on Windows) for `./check`. The same Rust builds everywhere: locally, in CI and in the Docker image.

## First setup

This loads the demo recipes (`fixtures/demo-bundle`, written for mise) into a database of their own, `mise_demo`,
which `.env.example` points at.

```bash
cp .env.example .env
docker compose -f compose.dev.yaml up -d db      # Postgres 16 on localhost:5433, user and password mise
docker compose -f compose.dev.yaml exec db createdb -U mise mise_demo
cargo build --release -p mise-cli -p mise-api
M=./target/release/mise
$M migrate
$M import-bundle fixtures/demo-bundle
$M match-foods
$M food apply fixtures/demo-bundle/curation.json
$M group-dishes
$M derive-options
$M parse-steps
cd web && npm install && npm run build && cd ..
git config core.hooksPath .githooks               # optional: formatting and lints before each commit
```

Then `./target/release/mise-server` serves the app on :8080, and the first visit creates the admin account.

## Databases

| Database | What | Used for |
| --- | --- | --- |
| `mise_demo` | The demo recipes (First setup above) | Development (`.env.example` points here) |
| `mise_test_<uuid>` | Made and dropped by each API integration test | Tests only |
| `mise_e2e` | Rebuilt from the demo recipes by each browser-journey run | The browser journeys only |
| `mise_sqlx` | Empty, migrated to this checkout's migrations by `scripts/sqlx-prepare` | Checking `mise-api`'s queries against the schema |
| `mise_seed` | Maintainer only: the launch seed, 237 dishes and 308 versions from meals the household received | Day-to-day use |
| `mise` | Maintainer only: the full private archive import, about 9,900 dishes | Evidence: package sizes, pack counts, more dishes to add later |

Switching between them is a matter of `DATABASE_URL` and `MEDIA_ROOT` in `.env`.

## Rebuilding the seed database (maintainer only)

The private seed lives in a separate private repository, cloned into the ignored `seed/` folder; its README says
how to restore the databases from snapshots. Rebuilding from scratch also needs the archive's photo folder
(`ARCHIVE` below) and Python 3 for the seed scripts. Nothing outside this section depends on any of it.

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
standard input) recovers a lost one. The OpenAPI document is at `/api/v1/openapi.json`.

A second server for trying things, without touching the one in daily use:

```bash
MISE_BIND=127.0.0.1:8091 MISE_WEB_DIR=web/dist ./target/debug/mise-server.exe
```

The service worker only registers in production builds (`npm run build`), and the built-in browser pane in the
Claude desktop app doesn't run it, so offline start-up has to be tried in a real browser.

## Tests

`./check` runs CI's build, lint and test steps in CI's order, and stops at the first failure, naming the step:

```bash
./check            # Rust formatting, clippy, tests, docs, the API document check, then web formatting, lint, tests, build
./check --e2e      # the same, then the browser journeys
./check --fast     # formatting and lints only
```

It runs the test steps with `CI` set, as CI does. So the API tests fail when `DATABASE_URL` (from the environment or
`.env`, and passed on to the browser journeys) is missing or unreachable, rather than skipping, and Vitest and
Playwright refuse a focused `.only` test. As in CI, it regenerates `web/src/api/schema.d.ts` and fails if git shows a
change, leaving the new file to commit. It runs `npm ci` only when `web/node_modules` is missing or older than
`web/package-lock.json`. Without `--e2e` it skips the browser journeys. `cargo deny`, `npm audit`, the Docker
image build and publishing the docs stay CI-only: `./check` never runs them.

A pre-commit hook runs `./check --fast` on the working tree before each commit. Turn it on once per clone:

```bash
git config core.hooksPath .githooks
```

`git commit --no-verify` skips it for one commit. The steps one by one:

| What | Command | Notes |
| --- | --- | --- |
| Rust formatting | `cargo fmt --all` (CI: `--check`) | rustfmt at its defaults (`rustfmt.toml`) |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` | Zero warnings |
| Rust unit, property and integration tests | `cargo test --workspace` | API tests and the pipeline snapshot test need `DATABASE_URL` and make a throwaway database each; skipped when it isn't set |
| Rust docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | A broken intra-doc link or bad doc markup fails it; add `--open` to read them |
| Rust dependencies | `cargo deny check` | Advisories, licenses, bans and sources (`deny.toml`); CI runs it |
| Web formatting and lints | `cd web && npm run format` and `npm run lint` | Prettier at width 100; ESLint with the React hooks rules and a doc comment on every export |
| Web unit tests | `cd web && npm test` | Vitest; pure modules (`choose.ts`, `offline.ts`, `recipe.ts`, `week.ts`) |
| Types and build | `cd web && npm run build` | Runs the TypeScript check first |
| Browser journeys | `cd web && npm run e2e` | Playwright in Chromium against `mise-server` on a fresh `mise_e2e` database built from the demo recipes (`fixtures/demo-bundle`). Build first: `cargo build -p mise-cli -p mise-api` and `npm run build`; once, `npx playwright install chromium`. Covers first-run setup, scaling, recipe cards (the dish page's amounts, one printed page), planning (the Phase 3 gate: under 20 interactions), shopping offline including opening with no connection, a kitchen screen at 1280 × 800 (a step's timer counts down on a fake clock), phone width, an axe accessibility scan and sign-out. The browser's clock is set to a Sunday evening in New York, when UTC is already Monday, so its week is never the server's own and anything that leans on the server's default week fails on every run |
| Scaling report | `mise scaling-report --sample 300 --out reports/scaling.html` | Compares derived amounts with what the source published; read-only |
| Grouping report | `mise gate-report` | A stratified sample of dishes to check grouping by hand |

The fuzz suite in `crates/mise-core/tests/fuzz.rs` takes about 25 seconds in a debug build. The plan for the
remaining repeatable gate (a seed gate) is in [backlog.md](backlog.md).

### Pipeline snapshots

`crates/mise-cli/tests/pipeline_snapshot.rs` runs the demo recipes through every import stage in a throwaway database
(like the API tests, it needs `DATABASE_URL`) and compares the result with the plain-text files in
`fixtures/demo-bundle/snapshots/`: `pipeline.txt` holds each stage's report, the row counts, the open review items
and the food tree, and each `<dish slug>.txt` holds the dish page and every version at 1 to 6 servings as the API
renders it. A change to food matching, grouping, options, step parsing or rounding, or to the demo bundle itself,
makes it fail with a unified diff whose hunk headers name the dish version and serving count:

```text
@@ -92,7 +92,7 @@ ### Version 1 at 5 servings (derived)
-- ½ cup + 2 TBSP Parmesan [scaled]
+- ⅔ cup Parmesan [scaled]
```

When the change is meant, accept it by rewriting the snapshots, then review them as you would code:

```bash
MISE_UPDATE_SNAPSHOTS=1 cargo test -p mise-cli --test pipeline_snapshot
git diff fixtures/demo-bundle/snapshots/    # every changed line should be one the change explains
```

The rewrite also adds a file for a new dish and removes the file of one that is gone. Commit the snapshots with the
change that caused them, and say in the merge request why they moved; a reviewer reads that diff to see what the
change does to real recipes.

## CI

`.gitlab-ci.yml` runs on every merge request and every push to `main`, on the project's GitLab runner, and a merge
request merges only when it passes. `.github/workflows/ci.yml` runs the same steps again on the GitHub mirror after
each push to `main`, as a second opinion. Both run what `./check` runs, with the same commands; change all three
together. Since the other jobs already prove the code builds, the Docker image job runs on a merge request only when
something the image build or the stack depends on changes (the Dockerfile, the Cargo and npm manifests and lockfiles,
the Rust version, `compose.yaml`, the `Caddyfile`, the pipeline itself); on `main` and tags it always runs. The Rust caches are keyed on the Rust version, not
`Cargo.lock`, so a dependency update rebuilds only what it changed.

### CI runners

Most jobs run on any Docker runner. The Docker image job builds the image and starts the whole stack, so it needs a
Docker of its own to talk to; it takes a runner tagged `docker-socket`, which lends jobs its own Docker through the
socket instead of running Docker-in-Docker. That needs no privileged containers, which matters on runners in
unprivileged Proxmox LXCs (with nesting on, so Docker runs inside), and the build cache stays between runs. A job
there can control that Docker, so give the tag only to runners whose Docker serves CI and nothing else. In the
runner's `config.toml`:

```toml
[runners.docker]
  privileged = false
  volumes = ["/var/run/docker.sock:/var/run/docker.sock", "/cache"]
```

Then tag the runner `docker-socket` (Admin, CI/CD, Runners), leaving "Run untagged jobs" on so it takes the other
jobs too. A fork's pipeline needs a runner set up the same way for this one job.

A Docker runner's disk fills unless something prunes it, and no job does that for the host. The security scanners'
images sit on tags GitLab moves every day or two (`gitlab-advanced-sast:2` is 1.1 GB), and each pull leaves the
old copy behind as a `<none>` image; on a 50 GB runner that filled the disk in under two weeks (2026-11). The job
caches grow too: each is a Docker volume, one per parallel job slot, holding the Rust `target/` archive and the
downloaded crates, and nothing trims them (GitLab's "Clear runner caches" starts new ones beside the old). On a
140 GB runner, 26 of them reached 92 GB (2026-11-24). Each runner host runs this from cron, in
`/etc/cron.d/docker-prune`:

```text
# Old scanner images left behind when their tags move (no run uses them again), daily.
15 4 * * * root docker image prune -f >/dev/null 2>&1
# Build cache not used in a week; anything a recent run touched stays.
45 4 * * * root docker builder prune -f --filter until=168h >/dev/null 2>&1
# The runner's own cache volumes and containers no job is using, weekly (Sunday): they only grow, and jobs rebuild them.
30 5 * * 0 root /usr/share/gitlab-runner/clear-docker-cache >/dev/null 2>&1
```

`clear-docker-cache` comes with the gitlab-runner package, and GitLab's Docker executor documentation recommends
running it from cron about once a week. It prunes only what carries the runner's label
(`com.gitlab.gitlab-runner.managed=true`), and passes `--all` to `docker volume prune` on Docker 23 and later, which
the runner's named cache volumes need. A running job's volumes are in use, so it never pulls one out from under a
job; the cost is one cold Rust build a week on each runner.

Images are pruned by whether a tag still points at them, not by age: an image's age is when it was built upstream,
so a time limit would delete `rust:1.100.0-bookworm` while every job still uses it. Build cache is pruned by when it
was last used. This is the runner hosts' setup rather than mise's, and belongs in the household's systems
documentation once that exists; it is here until then.

Security scanning comes from two places. The project's security configuration profiles (Secure, Security configuration;
not in `.gitlab-ci.yml`) add GitLab Advanced SAST, dependency scanning and secret detection to every merge request and
push to `main`. Advanced SAST doesn't read Rust, so the pipeline's own `rust sast` job runs Semgrep's Rust rules
(`p/rust`) and reports in GitLab's SAST format, alongside the others. Findings appear in the merge request's security
widget and the project's vulnerability report; scanners never fail the pipeline. A merge request approval policy, kept
in `Personal/mise-security-policy-project`, holds a merge request that adds a new high or critical SAST finding until
the maintainer approves it, and holds one whose pipeline has no SAST results. Authors may approve their own merge
requests, since the project has one maintainer: the approval is the deliberate look at the finding. Fix a real finding;
dismiss a false positive in the vulnerability report with the reason.

The steps: the Rust job (formatting, clippy, tests against a Postgres service, among them one that every route
answers only with the statuses the OpenAPI document lists for it, a check that every response in the
OpenAPI document has the description OpenAPI 3.1 requires (`web/scripts/check-openapi.mjs`), and a check that
`web/src/api/schema.d.ts` matches the server), the Rust docs built with warnings as errors (`rust docs` on merge
requests; on `main`, `pages` builds them once and publishes them to GitLab Pages, opening at `mise_core`), the web job
(formatting, lint, tests, build, an audit of shipped npm packages), the browser journeys (keeping Playwright's report
and traces when they fail), `cargo deny`, and a build of the Docker image.

Renovate keeps dependencies current. A daily pipeline schedule on `main` (with the variable `SCHEDULE=dependencies`)
runs the `renovate` job. Ordinary updates come on Mondays (UTC), one merge request per ecosystem as `renovate.json`
groups them (Rust crates, web packages, container images, the mirror's GitHub Actions). A dependency with a known
vulnerability in the OSV database gets its fix on the next daily run, labelled `security`. Renovate only updates
dependencies the manifests name, so a vulnerable transitive crate or package (one the security scans report) is fixed by
hand with `cargo update -p <crate>` or `npm update`. Rust (ADR 0007) and Postgres majors, which need the data upgraded,
stay raised by hand. TypeScript majors wait on approval in Renovate's Dependency Dashboard issue until every tool that
depends on TypeScript accepts them: the same schedule runs `typescript watch` (`npm run typescript:ready` in `web/`),
which opens an issue when that happens. The `renovate config` job checks `renovate.json` whenever it changes. Mira
(`@mira`), the maintainer's self-hosted AI reviewer, reviews every merge request.

`.git-blame-ignore-revs` lists the formatting-only commits; run `git config blame.ignoreRevsFile .git-blame-ignore-revs`
once per clone.

## Releasing

A release is a version tag on `main`. Its pipeline runs every check, then publishes (`release image` and `release`
in `.gitlab-ci.yml`): the Docker image goes to the container registry as the version (`0.1.0`) and, unless it is a
pre-release, as `latest`, and a GitLab release is made with the version's section of `CHANGELOG.md` as its notes.

1. In a merge request: move what `## [Unreleased]` in `CHANGELOG.md` holds under a new `## [x.y.z] - <date>`
   heading (and its link at the end), and set `version` in every crate's `Cargo.toml` to it. The release jobs refuse
   a tag that doesn't match `crates/mise-api/Cargo.toml`.
2. Once that is merged, tag `main` and push the tag: `git tag -a vx.y.z -m "mise x.y.z" origin/main && git push
   origin vx.y.z`. Protect `v*` tags (**Settings → Repository → Protected tags**, maintainers only) so nobody else,
   a bot included, can publish one.
3. A pre-release (`vx.y.z-rc.1`) publishes its image as `x.y.z-rc.1` and leaves `latest` alone: use one to try the
   release path. Delete a trial release and its image afterwards (**Deploy → Releases**, **Deploy → Container
   registry**), and its tag.

## Changing the API

Add the handler with a `#[utoipa::path]` attribute, register it in `ApiDoc` and the router in `lib.rs`, add an
integration test in `crates/mise-api/tests/`, then regenerate the web types (`cd web && npm run api`) and add the
type alias to `web/src/api/client.ts`.

## Changing a query

`mise-api`'s queries are sqlx's checked macros (`query!`, `query_as!`, `query_scalar!`): each is compared with the
schema when it compiles, so a column that doesn't exist or a type that doesn't fit fails the build. The build reads
what it compares against from `crates/mise-api/.sqlx`, committed, so it needs no database (`.cargo/config.toml` sets
`SQLX_OFFLINE`, and the Dockerfile does too).

After changing a query in `mise-api`, or adding a migration, refresh that metadata and commit what changes:

```bash
scripts/sqlx-prepare           # migrates mise_sqlx on the dev Postgres and rewrites crates/mise-api/.sqlx
scripts/sqlx-prepare --check   # fails when the committed metadata no longer matches (./check and CI run this)
```

It works on `mise_sqlx`, a database of its own on the Postgres `DATABASE_URL` names, made when missing and migrated
to the checkout's migrations each time, so it never touches a database in use (`SQLX_DATABASE_URL` names another).
While writing a query, `SQLX_OFFLINE=false DATABASE_URL=<…>/mise_sqlx cargo check -p mise-api` checks against the
database directly, before the metadata exists.

A query that must be built at run time (a dynamic `ORDER BY`, say) keeps sqlx's unchecked functions, with a comment
saying why. None does today. Postgres can't always tell whether a computed column can be null (a `count(*)`, an
`EXISTS`, a cast); the column's alias says so for it, `AS "total!"` for never null and `AS "total?"` for maybe, and
nothing else about its type is overridden, so the type is still checked.

## Changing the schema

Add `migrations/00NN_<what>.sql` with a first-line comment saying what it adds. The server and `mise migrate` apply
it. An older binary refuses to start against a database that has a migration it doesn't know, so rebuild before
restarting after pulling. Then run `scripts/sqlx-prepare` (above), since the queries are checked against the new
schema.
