# Architecture decision records

The decisions a reviewer is most likely to ask about. Each record gives the context, the decision, the
alternatives and why they lost, and what follows from it. [status.md](../status.md) keeps the full list of
decisions, one line each. [architecture.md](../architecture.md) describes how the system works today.

These records were written on 2026-11-04, after the decisions were made. Where the reasoning was written down at the
time (the design document, a commit message, a review), the record says where. Where it wasn't, the record says
what the code and docs show and doesn't make up a debate that never took place.

| No. | Decision | Date |
| --- | --- | --- |
| [0001](0001-build-our-own-app.md) | Build our own app rather than adapt Mealie or Tandoor | 2026-10 |
| [0002](0002-rust-axum-sqlx.md) | Rust, with axum for HTTP and sqlx for the database | 2026-10 |
| [0003](0003-postgres.md) | Postgres as the one database | 2026-10 |
| [0004](0004-generated-api-client.md) | React, with a client generated from the server's OpenAPI document | 2026-10 |
| [0005](0005-offline-shopping-list.md) | The shopping list works offline from a device copy and queued changes | 2026-11-03 |
| [0006](0006-round-up-into-measures.md) | Scaled amounts round up into standard measures | 2026-11-02 |
| [0007](0007-one-pinned-rust.md) | One pinned Rust for local builds, CI and Docker | 2026-11-04 |

A new record takes the next number and the same shape: context, decision, alternatives, consequences. A decision
that replaces an earlier one gets a new record, and the old one is marked "Replaced by NNNN" rather than rewritten.
