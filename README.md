# mise

[![CI](https://github.com/poiu477/mise/actions/workflows/ci.yml/badge.svg)](https://github.com/poiu477/mise/actions/workflows/ci.yml)

A self-hosted recipe system: dishes with versions, variant toggles, any-servings scaling, a shared weekly plan and
a shopping list that works offline. Recipes come from seed bundles, websites, files, or your own kitchen, and all get
the same treatment.

Status: phases 1 to 3 (data core; browse and cook; plan and shop) are done and in daily use. Next are the remaining
production-quality tasks, then kitchen mode and imports. See [docs/status.md](docs/status.md) for where things stand,
[docs/backlog.md](docs/backlog.md) for the work still to do, and [CLAUDE.md](CLAUDE.md) for how to work on the code.

## Running it

On a home server: `cp deploy.env.example .env`, set `POSTGRES_PASSWORD`, then `docker compose up -d --build`.
See [docs/deploy.md](docs/deploy.md).

## Development

Requirements: Rust 1.99.1 (pinned in `rust-toolchain.toml`; rustup installs it), Node 26, Docker. The full guide, including rebuilding a seed database and running
the tests, is [docs/development.md](docs/development.md).

```bash
cp .env.example .env
docker compose -f compose.dev.yaml up -d db
M="cargo run -q --release -p mise-cli --"
$M migrate
$M import-bundle fixtures/demo-bundle   # four demo dishes written for mise
$M match-foods
$M food apply fixtures/demo-bundle/curation.json
$M group-dishes
$M derive-options
$M parse-steps
cargo run --release -p mise-api        # API on :8080; OpenAPI at /api/openapi.json
```

The web app lives in `web/` (React, Vite, TypeScript). `npm run dev` serves it on :5173 with `/api` forwarded to the
API; `npm run build` writes `web/dist`, which `mise-server` serves on its own port. After changing the API, run
`npm run api` to regenerate the client types from the OpenAPI document.

```bash
cd web && npm install && npm run dev
```

Linux builds (matching the production container) run in Docker:

```bash
docker compose -f compose.dev.yaml run --rm rust cargo test
```

## Layout

| Path | What |
| --- | --- |
| `crates/mise-core` | Quantities, scaling and rounding, step-text amounts, shopping totals, text helpers |
| `crates/mise-db` | Connection pool and embedded migrations |
| `crates/mise-ingest` | Importing and curating: seed bundles, food matching, dish grouping, options, media, steps |
| `crates/mise-api` | The REST API (`mise-server`) |
| `crates/mise-cli` | The `mise` admin command |
| `web/` | The web app (installable PWA) |
| `migrations/` | Postgres schema |
| `fixtures/demo-bundle` | Demo recipes for tests and trying mise out |
| `web/e2e/` | Browser journeys (Playwright) |
| `docs/status.md` | Where things stand, known issues, open questions, decisions |
| `docs/backlog.md` | The work still to do, as self-contained tasks |
| `docs/architecture.md` | How the parts fit together |
| `docs/development.md` | Setup, databases, running, testing |
| `docs/style.md` | How code, comments, commits and UI text are written |
| `docs/curation.md` | Food curation files, dish decisions, preferred versions |
| `docs/seed-bundle.md` | The bulk-import format |
| `docs/sign-in.md` | Accounts, households, single sign-on, API tokens |
| `docs/scaling-rules.md` | How recipes scale: established rules, exceptions, the archive's evidence |
| `docs/deploy.md` | Running mise with Docker Compose (e.g. in a Proxmox LXC) |
| `compose.yaml`, `Dockerfile` | The production stack; settings in `deploy.env.example` |
