# mise

A self-hosted recipe system: dishes with versions, variant toggles, any-servings scaling, a shared weekly plan and
shopping list. Recipes come from seed bundles, websites, files, or your own kitchen, and all get the same treatment.

Status: phase 1 (data core) done; phase 2 (browse and cook) in progress. See the design document for the full plan.

## Development

Requirements: Rust (stable), Node 20+, Docker.

```bash
cp .env.example .env
docker compose -f compose.dev.yaml up -d db
cargo run -p mise-cli -- migrate
cargo run --release -p mise-cli -- import-bundle path/to/bundle
cargo run --release -p mise-cli -- parse-steps
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
| `crates/mise-core` | Domain types, quantities and scaling, text helpers |
| `crates/mise-db` | Connection pool and embedded migrations |
| `crates/mise-ingest` | Importers (seed bundles today) |
| `crates/mise-api` | The REST API (`mise-server`) |
| `crates/mise-cli` | The `mise` admin command |
| `web/` | The web app (installable PWA) |
| `migrations/` | Postgres schema |
| `docs/seed-bundle.md` | The bulk-import format |
