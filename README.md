# mise

A self-hosted recipe system: dishes with versions, variant toggles, any-servings scaling, a shared weekly plan and
shopping list. Recipes come from seed bundles, websites, files, or your own kitchen, and all get the same treatment.

Status: phase 1 (data core) in progress. See the design document for the full plan.

## Development

Requirements: Rust (stable), Docker.

```bash
cp .env.example .env
docker compose -f compose.dev.yaml up -d db
cargo run -p mise-cli -- migrate
cargo run --release -p mise-cli -- import-bundle path/to/bundle
```

Linux builds (matching the production container) run in Docker:

```bash
docker compose -f compose.dev.yaml run --rm rust cargo test
```

## Layout

| Path | What |
| --- | --- |
| `crates/mise-core` | Domain types and text helpers |
| `crates/mise-db` | Connection pool and embedded migrations |
| `crates/mise-ingest` | Importers (seed bundles today) |
| `crates/mise-cli` | The `mise` admin command |
| `migrations/` | Postgres schema |
| `docs/seed-bundle.md` | The bulk-import format |
