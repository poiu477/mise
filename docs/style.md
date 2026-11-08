# Style

Write like the code around you: match its comment density, naming and idiom. What follows is what that code does.

## Rust

- **Layout.** Each file opens with a `//!` comment saying what it holds in a sentence or two. rustfmt at its
  defaults decides the rest; CI rejects anything `cargo fmt` would change, and clippy runs with warnings as errors.
- **Comments.** A `///` comment on every public item says what it is or does, in plain English, often with an example
  in quotes: `/// "1½", "¾", "2"; falls back to a trimmed decimal`. Inline comments explain *why* (a source quirk, a
  rule's evidence), never what the next line plainly does. Few of them.
- **Names.** Plain words for what things are in the kitchen: `measured`, `line_text`, `keep_consistent`, `Group`,
  `CountUnit`. No `Manager`, `Helper` or `Util`.
- **Errors.** `anyhow` with `.context(...)` in the CLI and ingest; `ApiError` (`error.rs`) in the API, whose messages
  are shown to people, so they are sentences ("enter between 1 and 200 characters").
- **SQL.** Today: inline strings with `sqlx::query`, `query_as` and `query_scalar` plus `.bind(...)`, rows as
  tuples with a `type` alias naming their columns, inside the handlers. The target (in the backlog): rows as
  `#[derive(sqlx::FromRow)]` structs, compile-time checked `query_as!` with the `.sqlx` offline data committed, and
  SQL in per-domain data modules the handlers call. New code should move that way. Ids are UUID v7 made in Rust
  (`Uuid::now_v7()`).
- **Pure logic in `mise-core`.** Anything that can be a function of its inputs lives there with unit tests; the API
  and ingest crates do I/O and call it.
- **Tests.** Names read as the rule they check (`counts_carry_their_size`, `a_household_sees_only_its_own_plan`), and
  assertions compare what a person would see (`"1 lb 14 oz"`). Unit tests sit in a `mod tests` at the end of the file;
  integration tests in `crates/<crate>/tests/` build their own throwaway database and their own made-up foods.

## Migrations

One numbered file per change, opening with a comment on what it adds, and a comment on each table. snake_case,
`uuid` keys, `timestamptz`, closed sets as `text` with a `CHECK` constraint. Never edit an applied migration.

## TypeScript and React

- Function components and hooks; no state library. Server data loads through TanStack Query (`api/query.ts`)
  calling the typed `api` client, never by setting state in an effect (ESLint enforces it). Data that is an
  account's own has the account in its query key. Endpoints that change things answer with the whole updated
  object, which replaces the cached data (`queryClient.setQueryData`).
- A ref counter keeps only the latest answer when quick taps race (see `ShoppingPage.tsx`).
- Pure logic goes in a plain module with a Vitest test next to it (`week.ts`, `offline.ts`, `choose.ts`).
- Prettier (width 100) formats everything and ESLint (recommended sets plus the React hooks rules) lints it; CI
  rejects either failing.
- Styling is one `styles.css` with design tokens as CSS variables (`--accent`, `--line`, `--muted`), light and dark.
  No UI library.
- Every page works at 375 px wide with no sideways scroll. Icon-only buttons carry an `aria-label`; form fields have
  labels.

## Words people read

UI text, error messages, docs and commit messages share one voice: short, plain, specific.

- Say what happened and what to do: "Can't reach the server, so that wasn't saved. Ticks and added items still work
  offline."
- Sentence case for headings and buttons ("Add to plan", "Always have"). Kitchen style for amounts: ½ and ¼ glyphs,
  `TBSP`, `tsp`, `oz`, `lb`, "1 packet (1½ oz)".
- Name the household's things the way they do (dish, version, servings, list), not the schema's.
- Docs open with the point, then tables for facts. Sentences under about 25 words.

## Commits

- Subject: the outcome in plain words, no prefix, no period: "Plan a week and shop it from a merged list".
- Body: a few short paragraphs of prose on what changed for the person using it, then how, wrapped near 76 columns.
- Trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` (or the model that did the work).
- Stage files by name; commit only when asked.
