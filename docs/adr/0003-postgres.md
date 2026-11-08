# 0003 · Postgres as the one database

Accepted, 2026-10. The reasoning is recorded in the design document ("Why Postgres").

## Context

Two programs write to the database: the server (`mise-server`), as people plan and tick, and the admin command
(`mise`), which imports and curates data while the server runs. There is no separate worker yet. Search has to
forgive typos in dish names, and importing matches each ingredient name to a food by how alike the names are. Each
source's raw record is kept as it came, so an import can be processed again
([architecture.md](../architecture.md#data-model)).

## Decision

Postgres 16, run as one more container in the Compose stack ([deploy.md](../deploy.md)), or an external server
through `DATABASE_URL`. The schema uses what Postgres gives directly:

- `pg_trgm` with GIN indexes on dish titles, food names and aliases
  ([0001_data_core.sql](../../migrations/0001_data_core.sql)); dish search ranks by `similarity()`, and grouping
  and curation use it too.
- `jsonb` for each source record's raw data and for free-form properties (`foods.props`, a version's `meta`).
- `uuid` keys made in Rust and `timestamptz`. Closed sets are Postgres enums in the first migration (`food_kind`,
  `review_status` and three more) and `text` with a `CHECK` since, which is the style for new ones
  ([style.md](../style.md#migrations)).

## Alternatives

- **SQLite.** One file and no server, which suits one household. The design document's reason against it: the
  server and the import write at the same time, trigram search serves both dish search and food matching, and
  `jsonb` keeps each raw record. SQLite would work for one household but loses these (it takes one writer at a
  time and has nothing like `pg_trgm`'s `similarity()` ranking), and Postgres costs only one more container.
- **A document store.** Dishes, versions, ingredient lines, foods and plans are relational, with foreign keys
  between them that the schema enforces; a document store would leave those checks to the code.

## Consequences

- Development and tests need a running Postgres (`compose.dev.yaml`, port 5433). API tests each create a throwaway
  database, so they can run side by side; CI runs a Postgres service.
- Backups are a nightly database dump from the Compose stack, kept for `BACKUP_DAYS` (14 by default).
- Every migration is plain SQL, numbered and never edited once applied, because sqlx checksums them.
