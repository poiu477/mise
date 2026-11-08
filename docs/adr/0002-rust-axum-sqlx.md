# 0002 · Rust, with axum for HTTP and sqlx for the database

Accepted, 2026-10, as part of "Build our own app (Rust, Postgres, React)" in status.md's Decisions. The design
document names the stack (one Rust workspace with axum, sqlx and utoipa) but doesn't record why it beat others, so
this record sets out what the code relies on.

## Context

The server holds the parts that must be right every time: parsing and converting quantities, scaling to any serving
count, rounding into measures ([0006](0006-round-up-into-measures.md)), rewriting step text, adding up the shopping
list. It also has to run unattended in a small container next to Postgres, and serve the web app and an API from
one process. The offline seed tooling from the pilot was Python and stays outside the app.

## Decision

- **Rust**, as one workspace of five crates ([architecture.md](../architecture.md)). Pure logic lives in
  `mise-core` with no I/O, so it is tested as plain functions; the API and ingest crates do the I/O and call it.
- **axum** for HTTP. Sign-in is one middleware (`auth::require_auth`, added with `from_fn_with_state`) in front of
  every route, and the rest comes from tower-http layers: static files, compression, tracing, headers.
- **sqlx** for the database: async, Postgres-native, SQL written by hand. The migrations are embedded in `mise-db`
  and the server applies them when it starts.
- **utoipa** for the OpenAPI document, generated from annotations on the handlers
  ([0004](0004-generated-api-client.md)).

## Alternatives

- **Python (FastAPI or Django), like the pilot scripts.** Faster to start and closer to the tooling that already
  existed. It loses on what `mise-core` needs most: units and quantities as types the compiler checks
  (`Unit::Count(String)` is not `Unit::Tsp`), a `match` over units that fails to compile when a unit is missed,
  and one self-contained binary in the image.
- **TypeScript on Node, sharing types with the web app.** It would remove the generated client, but the quantity
  logic would rest on one floating-point number type, and checking every unit is handled would be a convention
  rather than the compiler's default.
- **actix-web instead of axum.** Equally capable. axum wins on fit: it is built on tower, so the middleware above
  is the stock tower-http set, and utoipa supports it directly (`axum_extras`).
- **An ORM (Diesel or SeaORM) instead of sqlx.** The queries lean on Postgres itself: trigram `similarity()` for
  search, `jsonb` fields, `CHECK` constraints on closed sets. Hand-written SQL says exactly that; an ORM would add
  a schema layer to keep in step with the migrations and still need raw SQL for those parts.

## Consequences

- `mise-core` carries most of the tests, including a fuzz suite that puts 100,000 random amounts through the
  rounding (`crates/mise-core/tests/fuzz.rs`), and they run without a database.
- Builds are slow, and on Windows a running server locks its binary, so a second server runs as a debug build on
  another port ([development.md](../development.md#running)).
- Today the SQL is inline strings checked only at run time, with rows as tuples ([style.md](../style.md#rust)). The
  target is `#[derive(sqlx::FromRow)]` structs and compile-time checked queries with the `.sqlx` data committed;
  that work is on the task list in [status.md](../status.md).
- The toolchain is pinned so every build uses the same compiler and lints ([0007](0007-one-pinned-rust.md)).
