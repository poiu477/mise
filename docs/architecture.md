# Architecture

One Rust workspace and one web app. The server (`mise-server`) answers the REST API and serves the built web app;
the admin command (`mise`) imports and curates data. Both talk to one Postgres database. Why it is built this way,
and what else was weighed, is in the [architecture decision records](adr/README.md).

```text
web/ (React PWA) ──HTTP──> mise-api (axum) ──> mise-core (pure logic)
                                 │                  ▲
                                 └──> mise-db ──> Postgres <── mise-ingest <── mise-cli (`mise`)
```

| Crate | Holds | Depends on |
| --- | --- | --- |
| `mise-core` | Pure logic, no I/O: quantities and units, scaling, rounding into standard measures, step-text templates, shopping totals, dish titles, food names | serde, regex |
| `mise-db` | The connection pool and the embedded migrations | sqlx |
| `mise-ingest` | Getting data in: seed bundles, food matching, curation files, dish grouping and decisions, options, media, step parsing, pruning | core, db |
| `mise-api` | The REST API and `mise-server` binary: auth, dishes, versions, media, plan, shopping; OpenAPI via utoipa | core, db |
| `mise-cli` | The `mise` binary: wraps ingest, plus food/dish/user commands and the gate and scaling reports | core, db, ingest |

## Data model

Migrations in `migrations/` are the schema of record; each file starts with a comment saying what it adds.

- **Sources and records.** A `source` (a seed bundle, the web, a file, a person) supplies `source_records`, kept
  verbatim so imports can be re-processed. Each record becomes a `variant`.
- **Dishes and variants.** A `dish` is what people choose; its `variants` are the published or forked versions.
  `variant_yields` are the serving counts a version was published for. `dish_options` (protein, portion, version) are
  derived from the differences between a dish's variants. `dish_decisions` (merge, keep apart) and
  `preferred_versions` (how often each version was had) steer grouping and the default version.
- **Ingredients.** Each `ingredient_line` belongs to a variant and points at one `food`; `line_amounts` hold its
  amount at each published yield. `shipped = false` marks pantry lines (salt, oil) the household is expected to have.
- **Foods.** A single-parent tree (`foods.parent_id`) with `food_aliases`, `food_relations` and `food_facts`. Free-form
  properties live in `foods.props`: `count_unit` and `count_size` (how "1 unit" reads), `store_section`,
  `default_flexibility`.
- **Steps.** `steps` hold the published text and, after `mise parse-steps`, a template (JSON) that can be rendered at
  any serving count.
- **Media.** Content-addressed `media` (by hash) with `media_derivatives` (web sizes) and links to variants, steps
  and foods.
- **Review.** `review_items` queue doubtful merges, unmatched names and conflicting evidence for a person.
- **Accounts.** `households` (the sharing boundary), `users`, `user_identities` (OIDC), `sessions`, `api_tokens`.
- **Planning.** `plan_entries` (a version and serving count in a household's week, named by its Monday);
  `shopping_marks` (ticked or removed items), `shopping_extras` (added by hand), `pantry_staples` (always on hand),
  `food_weight_units` (a food read in other units than the household's `weight_unit`).

## Getting data in

A seed bundle (see [seed-bundle.md](seed-bundle.md)) is built offline from a private archive and imported with
`mise import-bundle`. Then, in order: `match-foods` (one food per ingredient name, a proposed tree, review items),
`food apply` (curation file), `dish apply` and `dish prefer` (decisions and preferred versions), `group-dishes`,
`derive-options`, `parse-steps`, `ingest-media`. Every stage is re-runnable and leaves what a person decided alone.

**Grouping** joins variants into dishes: records the source marks as cloned from each other first, then similar
titles; chains of small changes that drift into a different dish are split; protein swaps of one dish are joined.
Doubtful merges go to review, and recorded decisions always win.

## Scaling and measuring (`mise-core`)

- `quantity.rs`: `Unit` and `Quantity` (parse, convert within a family, display in the house style "1½ TBSP").
  `Quantity::measured()` rounds a derived amount **up** into standard measures: one cup, spoon or pinch where that is
  at most 5% over, else the fewest measures that come closest ("1 tsp + 1 pinch"); weights to the next ½ oz, counts
  to the next half. `exact()` prints an amount as it is, for the shopping list.
- `servings.rs`: the amount at any serving count. A published yield is used exactly; between yields amounts are
  interpolated, outside them scaled from the nearest; `keep_consistent` drops published amounts that would shrink as
  servings grow. `line_text` words an ingredient line; `line_quantity` gives the number the shopping list adds up.
  `CountUnit` turns a bare "1 unit" into "1 packet (1½ oz)".
- `step_amounts.rs`: step text parsed into parts (text, amounts linked to ingredient lines, worded alternatives,
  notes for larger yields) and rendered for a serving count, flagging amounts to check. Pan-sauce water keeps a
  fixed boil-off share once per pan. The rules and their evidence are in [scaling-rules.md](scaling-rules.md).
- `shopping.rs`: `total()` adds a food's needs exactly, weights in a `WeightUnit` (lb + oz, oz, lb, metric), counted
  items folded into a measure where their size is known; `SECTIONS` is the walking order of a shop.

## The API (`mise-api`)

`lib.rs` builds the router; every route sits behind `auth::require_auth` except health, the OpenAPI document and
sign-in. A session cookie (`mise_session`) or a bearer token (`mise_…`, read or read-write) identifies a
`CurrentUser` with their `household_id`, and every planning query is scoped by it. The web app also sends
`X-Mise-Account` with the account it is working as; when that isn't the account signed in (someone else signed in
since, in another tab, with the shared cookie) the request is refused with 412, so queued work never lands in
another household, and the app re-checks who is signed in. Handlers return `ApiResult<T>`;
`error.rs` maps errors to JSON `{ "error": "…" }` with the right status. `extract.rs` wraps axum's extractors so bad
input reads as a 400 with a plain message. Endpoints that change a plan or a list answer with the whole plan or list,
so the client never has to merge.

The shopping list is never stored: `shopping::build` works it out from the week's plan on each request (lines grouped
by food, the food's store section from itself or its nearest ancestor, pantry lines under "Check you have") and lays
the stored marks, extras, staples and unit choices on top.

## The web app (`web/`)

React 19 with React Router, Vite and TypeScript. `api/client.ts` is an `openapi-fetch` client typed from the
generated `api/schema.d.ts`; pages load through TanStack Query (`api/query.ts`), which caches answers per page,
reloads them when a page comes back into view and keeps one account's data apart from another's. `session.tsx`
knows who is signed in (or, offline, who last was); `components/Layout.tsx`
gates pages on it. Pages: search, dish (options, servings, add to plan), plan, shopping, account, sign-in and first-run
setup.

Offline: `public/sw.js` keeps the app's files and recipe data (network first, cache as fallback; photos cache first)
and never caches account data. The shopping list keeps its own copy in `offline.ts`: each week's list as last seen,
plus ticks and hand-added items made without a connection, sent in order once the server answers. Everything kept on
the device is per account and cleared on sign-out. Service workers need HTTPS on a phone (or localhost).

## Deployment

`Dockerfile` builds one image with both binaries and the web app; `compose.yaml` runs it with Postgres and a nightly
`pg_dump` backup, and with the `https` profile, Caddy in front of it (`Caddyfile`). Settings are environment variables
with defaults (`deploy.env.example`); see [deploy.md](deploy.md). The server applies migrations when it starts.
