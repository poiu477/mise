# Architecture

One Rust workspace and one web app. The server (`mise-server`) answers the REST API and serves the built web app;
the admin command (`mise`) imports and curates data. Both talk to one Postgres database. Why it is built this way,
and what else was weighed, is in the [architecture decision records](adr/README.md).

```text
web/ (React PWA) ──HTTP──> mise-api (axum) ──> mise-core (pure logic)
                             │   │                  ▲
                             │   └──> mise-ingest <─┼── mise-cli (`mise`)
                             │             │        │
                             └──> mise-db ──> Postgres
```

| Crate | Holds | Depends on |
| --- | --- | --- |
| `mise-core` | Pure logic, no I/O: quantities and units, scaling, rounding into standard measures, step-text templates, ingredient lines read from text, shopping totals, dish titles, food names | serde, regex |
| `mise-db` | The connection pool and the embedded migrations | sqlx |
| `mise-ingest` | Getting data in: seed bundles, schema.org Recipe data from web pages and files, food matching, curation files, dish grouping and decisions, options, media, step parsing, pruning | core, db |
| `mise-api` | The REST API and `mise-server` binary: auth, dishes, versions, recipes written in the app, media, plan, shopping, review queues; OpenAPI via utoipa | core, db, ingest |
| `mise-cli` | The `mise` binary: wraps ingest, plus food/dish/user commands and the gate and scaling reports | core, db, ingest |

## Data model

Migrations in `migrations/` are the schema of record; each file starts with a comment saying what it adds.

- **Sources and records.** A `source` (a seed bundle, the web, a file, a person) supplies `source_records`, kept
  verbatim so imports can be re-processed. Each record becomes a `variant`. A record a household imported (a web
  page) belongs to it (`household_id`), like the version made of it; a bundle's belongs to none.
- **Dishes and variants.** A `dish` is what people choose; its `variants` are the published or forked versions.
  A variant belongs to one household (`household_id`, with a `visibility`: `private`, `partners` or `install`) or,
  for seed and bundle versions, to none; the SQL function `visible_to(variant, household)` says whether a household
  can see it, from that level, the owning household's `household_partners` and the install's `sharing_cap`, and a
  dish shows to a household only through the versions it can see. Two more functions beside it are rules several
  queries share: `default_version(dish, household)`, the version a dish opens on for a household, and
  `known_food(food, household)`, whether a household may know of a food.
  `variant_yields` are the serving counts a version was published for. `dish_options` (protein, portion, version) are
  derived from the differences between a dish's variants. `dish_decisions` (merge, keep apart) and
  `preferred_versions` (how often each version was had) steer grouping and the default version. `retired_slugs` keep
  the addresses a dish used to have, so a renamed dish still opens from an old link. A recipe written in the app
  is a variant with `origin = 'user'` and no source record, in a dish of its own (`dishes.origin = 'user'`, the
  variant `dish_pinned`), which grouping leaves alone. A fork, the household's own version of another, is the same
  kind of variant in the original's dish, pinned there, with `parent_variant_id` naming the original.
- **Ingredients.** Each `ingredient_line` belongs to a variant and points at one `food`; `line_amounts` hold its
  amount at each published yield. `shipped = false` marks pantry lines (salt, oil) the household is expected to have.
  `text` keeps a line as it was typed ("2 cups flour, sifted") for a recipe written in the app.
- **Foods.** A single-parent tree (`foods.parent_id`) with `food_aliases`, `food_relations` and `food_facts`. Free-form
  properties live in `foods.props`: `count_unit` and `count_size` (how "1 unit" reads), `store_section`,
  `default_flexibility`, `shop_as_one` (its kinds share a shopping line). `food_measures` hold what a food's amounts
  convert by on the shopping list: a `density` (a volume and its weight) or an `item` size (the volume or weight of
  one clove, one onion), set by curation; kinds of a food use the food's own when they have none.
- **Steps.** `steps` hold the published text and, after `mise parse-steps`, a template (JSON) that can be rendered at
  any serving count.
- **Media.** Content-addressed `media` (by hash) with `media_derivatives` (web sizes) and links to variants, steps
  and foods. A photo has no owner of its own: it is the versions' that use it, and a food's picture is everyone's
  (taken only from versions no household owns).
- **Review.** `review_items` queue doubtful merges, unmatched names and conflicting evidence for a person. An admin
  answers food matches, food parents and dish merges on the web app's review page (`mise-api`'s `review` module),
  which applies the same curation operation or dish decision as the `mise` command would.
- **Accounts.** `households` (the sharing boundary, with the `default_visibility` of what each writes and forks;
  `household_partners` are the households each shares with), `users`, `user_identities` (OIDC), `sessions`,
  `api_tokens`, `kitchen_tokens` (a kitchen device's one-time link, then its cookie).
- **Planning.** `plan_entries` (a version and serving count in a household's week, named by its Monday);
  `shopping_marks` (ticked or removed, per food: the key is the food's id, or the name of an unmatched line),
  `shopping_extras` (added by hand), `pantry_staples` (always on hand),
  `food_weight_units` (a food read in other units than the household's `weight_unit`).

## Getting data in

A seed bundle (see [seed-bundle.md](seed-bundle.md)) is built offline from a private archive and imported with
`mise import-bundle`. Then, in order: `match-foods` (one food per ingredient name, a proposed tree, review items),
`food apply` (curation file), `dish apply` and `dish prefer` (decisions and preferred versions), `group-dishes`,
`derive-options`, `parse-steps`, `ingest-media`. Every stage is re-runnable and leaves what a person decided alone.
Matching's rules for relating names (plurals, brands, narrowing words) are in `mise_core::food_names`; the brands
themselves are data, the `brands` table, which curation edits and each matching run reads. Matching can't tell
that two names it doesn't know are one food, so afterwards it pairs the foods it made whose names look like one
food written twice and asks a person (`food_duplicate` review items, [curation.md](curation.md)).

Web pages and other apps' export files come in through the same stages. `schema_org::read_page`, `read_json`
and `read_value` turn schema.org Recipe data (JSON-LD, or microdata when a page has none) into records shaped like a
bundle's variant lines, without fetching anything; the sample pages in `fixtures/web-pages/` show what each kind of
markup gives. `import` stores them, for pages and files alike: it keeps each recipe as a source record owned by the
importing household and makes a version of it owned by the same household: lines read with
`mise_core::ingredient_line` (kept as written in `text`), steps parsed as they are stored, the photo stored as an
upload is, foods matched in the same transaction, then `group-dishes` and `derive-options` run once at the end (one
import at a time). An unchanged recipe changes nothing; a changed one is a new version whose parent is the previous.

`web::import_url` (behind `POST /api/v1/imports` and `mise import url`) fetches the page and its photo with
`fetch::Fetcher` and stores its recipes as the `web` source's. `files::import_file` and `import_path` (behind
`POST /api/v1/imports/file` and `mise import file`) unpack an export (a zip, zips inside it, a gzip, or a folder on
disk, within size and count limits checked as it unpacks), sort each JSON document into a format by its fields, and
read it with that format's module (`files::paprika`, `mealie`, `tandoor`, `nextcloud`; schema.org JSON goes straight
to `read_value`). Each app's recipe is turned into schema.org JSON-LD and read like a page's, so yields, times and
steps follow one set of rules, and the record keeps the app's own object as its `raw`. Each format is a source of its
own (`file:paprika`), and the photo is the one the file holds: Paprika's inline `photo_data`, or the `full`,
`original` or `image` picture beside a recipe in a folder that holds no other. `fixtures/import-files/` has an
export of each format written for mise, with the records each gives.

`fetch::Fetcher` refuses what an import must not reach: anything but `http` and `https`, credentials in the link,
ports below 1024 other than 80 and 443, and any host that is or resolves to an address off the public internet
(`fetch::is_public`: loopback, private, link-local, shared, documentation and reserved ranges, IPv4-mapped and NAT64
IPv6 judged by the IPv4 inside). It resolves the host once and connects only to the addresses it checked, follows
redirects itself (each checked, at most 5), and limits time, size and content type. `IMPORT_ALLOW_PRIVATE=true`
lifts the address and port checks.

A recipe written in the app (`mise-api`'s `recipes.rs`) skips the bundle and goes through the same pieces in one
transaction: each ingredient line is read from its text by `mise_core::ingredient_line`, the lines that name no
food are matched by `foods::match_foods_in` (the matching stage itself, which takes turns under an advisory lock so
two saves never make one food twice), and each step becomes a template through `steps::template`. Its photo goes
through `media::store_upload`: decoded, turned upright, encoded again and stored by the hash of what was encoded,
with its web sizes made at once, so nothing of the uploaded file but the picture is kept.

**Grouping** joins variants into dishes: records the source marks as cloned from each other first, then similar
titles; chains of small changes that drift into a different dish are split; protein swaps of one dish are joined.
Doubtful merges go to review, and recorded decisions always win. A dish keeps its id from run to run, and its slug
follows its name: when the name changes, the old slug is retired and still finds the dish, as does the slug of a dish
whose versions all joined another. A live dish's slug always wins over a retired one. A household's own versions
may join a dish everyone sees, but never one only another household can see, and a shared dish is named, and opens
on, versions everyone sees.

## Scaling and measuring (`mise-core`)

- `quantity.rs`: `Unit` and `Quantity` (parse, convert within a family, display in the house style "1½ TBSP").
  `Quantity::measured()` rounds a derived amount **up** into standard measures: one cup, spoon or pinch where that is
  at most 5% over, else the fewest measures that come closest ("1 tsp + 1 pinch"); weights to the next ½ oz, counts
  to the next half. `exact()` prints an amount as it is, for the shopping list.
- `ingredient_line.rs`: one ingredient line as a cook writes it, read into an amount, a unit, the food's name and
  a note: "1 (14 oz) can diced tomatoes, drained" is 1 can of "diced tomatoes", noted "14 oz; drained". A range
  gives its lower end, a bare number counts the food, and a line it can't take apart is all name.
- `servings.rs`: the amount at any serving count. A published yield is used exactly; between yields amounts are
  interpolated, outside them scaled from the nearest; `keep_consistent` drops published amounts that would shrink as
  servings grow. `line_text` words an ingredient line; `line_quantity` gives the number the shopping list adds up.
  `CountUnit` turns a bare "1 unit" into "1 packet (1½ oz)".
- `step_amounts.rs`: step text parsed into parts (text, amounts linked to ingredient lines, worded alternatives,
  notes for larger yields) and rendered for a serving count, flagging amounts to check. Pan-sauce water keeps a
  fixed boil-off share once per pan. The rules and their evidence are in [scaling-rules.md](scaling-rules.md).
- `step_times.rs`: the times a step asks for ("8-10 minutes", "1 hour 15 minutes", "a minute more"), with where
  the text says them, the seconds a timer runs (a range's lower end) and a range's upper end. The API sends them with
  each step, for kitchen mode's timers.
- `shopping.rs`: `Measures::total()` adds a line's needs exactly, weights in a `WeightUnit` (lb + oz, oz, lb, metric),
  converting across units only where the food's density or item sizes say how; `gather()` puts kinds of a food on
  its line; `foods_to_mark()` and `all_marked()` turn ticks on lines into ticks on foods and back; `SECTIONS` is the
  walking order of a shop.

## The API (`mise-api`)

`lib.rs` builds the router. Every route is served under `/api/v1`; the same routes answer under the old `/api` for
one more release, with a `Deprecation` header and a `Link` to the new address (`mark_deprecated`), except
`/api/health` and single sign-on's two redirects, which keep their addresses for good. Every route sits behind
`auth::require_auth` except health, the OpenAPI document and sign-in. A bearer token (`mise_…`, read or read-write),
else a session cookie (`mise_session`), else a kitchen device's cookie (`mise_kitchen`) identifies a `CurrentUser` with
their `household_id`, and every planning query is scoped by it. Recipe queries (search, the dish page, a version, the
plan and the shopping list) read only the versions `visible_to` that household: another household's version answers
404, like one that doesn't exist, and a dish two households share shows each its own share of versions and option
choices. A photo (`/media`) follows the versions that use it: it is served to a household that can see one of them,
a food's picture to everyone, and any other answers the 404 of a photo that doesn't exist. Photos are sent
`Cache-Control: private`, so no shared cache answers in the server's place. A kitchen device has no account: its
`CurrentUser` carries its kitchen token's id, its household and
`Via::Kitchen`, so its reads go through the same household scoping, and `require_auth` refuses it every route but the
few `auth::kitchen::may` lists (reading `/me`, the plan, dishes, versions and photos) before any handler runs; `/me`
answers it with `kitchen: true` and no username, email or household name. Its cookie comes from a one-time link an
admin issues (`/api/v1/auth/kitchen?code=…`, valid for an hour), and is renewed, at most hourly, as the device is
used. Sessions, API tokens, kitchen links and kitchen cookies are random 256-bit secrets stored only as SHA-256
hashes. The web app also sends `X-Mise-Account` with the account it is working as; when that isn't the account
signed in (someone else signed in since, in another tab, with the shared cookie) the request is refused with 412, so
queued work never lands in another household, and the app re-checks who is signed in. Handlers return
`ApiResult<T>`; `error.rs` maps errors to the right status and one JSON body, `{ "error": "…", "code": "…" }`: a
sentence to show people, and a stable name for programs (`not_found`, `bad_request`, `unauthorized`,
`bad_credentials`, `forbidden`, `method_not_allowed`, `conflict`, `account_changed`, `payload_too_large`,
`too_many_requests`, `internal`, `site_failed`: a page being imported didn't answer, and `unavailable`: the database
isn't answering). `extract.rs` wraps axum's extractors so bad input answers in that shape too, and
the router does the same for an unknown address and a method a route doesn't take. Each route's `#[utoipa::path]`
lists every status it can answer: its own, and the shared sets in `error.rs` (`SignedIn`, `ReadOnlyToken`,
`TooLarge`, `ServerError`) for what the middleware, the body extractor and a failing server answer;
`tests/responses.rs` checks each route gives all of them and nothing else. Endpoints that change a plan or a
list answer with the whole plan or list, so the client never has to merge.

Every query is one of sqlx's checked macros, compared with the schema when `mise-api` compiles, against the metadata
committed in `crates/mise-api/.sqlx` (docs/development.md, "Changing a query"). The larger domains keep their SQL in
a `queries` module beside their handlers (`plan/queries.rs`, `shopping/queries.rs`, `recipes/queries.rs`,
`auth/queries.rs` for accounts, sessions, tokens, kitchen devices and single sign-on): a function per statement,
answering with a row struct named for what it reads, or with the API type itself when the columns are its fields.
The handlers hold the rules and the shapes. The smaller handlers (dishes, versions, review, sharing, foods, media)
keep their few queries inline. A rule more than one query needs (`visible_to`, `default_version`, `known_food`)
lives in the database as a SQL function, so each query stays a single statement the compiler can check.

### Security headers and requests from other sites

Every response, a page of the web app or an API answer, carries the headers in `security.rs`: a
Content-Security-Policy that lets a page load scripts, styles, photos, fonts and its service worker from mise alone
(and `blob:` and `data:` images, for the editor's preview of a photo not yet sent), runs no inline script or style,
loads no plugin, posts no form elsewhere and lets no other site frame it (`frame-ancestors 'none'`, and
`X-Frame-Options: DENY` for browsers that don't read that); `X-Content-Type-Options: nosniff`, so a response is only
ever what its type says; and `Referrer-Policy: same-origin`, so a dish's address or a kitchen link goes to no other
site. The browser journeys run the built app under the policy, so a change that breaks it fails them.

Another site can't make a signed-in browser change anything in mise, for three reasons that each hold on their own:

- **The cookies.** The session and kitchen cookies are `HttpOnly` and `SameSite=Lax`: a browser sends them with a
  request another site's page makes only when that site is a sibling on the same domain (`apps.example.com` against
  `mise.example.com`), and with a top-level link someone follows, which only reads.
- **What a page on another site can send.** The server answers no CORS preflight, so a browser won't send another
  origin's PUT, PATCH or DELETE, or a POST with a JSON body, at all. Every route that takes a body asks for a type a
  form can't send: `application/json` (`extract::JsonBody`), an image type for a photo, an `application/` type for an
  export file. Two routes take no body: `POST /auth/logout` and `POST /variants/{id}/fork`.
- **The browser's own label.** `security::refuse_cross_site` refuses (403) any request but GET, HEAD and OPTIONS that
  a browser marks `Sec-Fetch-Site: cross-site` or `same-site`, before the session is looked up: this is what covers
  the two routes without a body, and a sibling on the same domain. A browser too old to send that label sends
  `Origin` with every such request, which must then be the host the request went to or `PUBLIC_BASE_URL`. A request
  with neither comes from no browser page (the `mise` command, a script with a token) and goes through. It is the
  check Go's `net/http` makes (`CrossOriginProtection`).

Bearer tokens are never sent by a browser on its own, so a request made with one can't be forged this way. No GET
changes anything, but for two that need a secret the other site doesn't have: a kitchen device's one-time link (its
code) and single sign-on's callback (the `state` it sent, and PKCE). `tests/security.rs` sends sign-out, a DELETE and
a fork the way another site's page would and checks each is refused and changes nothing.

Password sign-in is throttled per username (`auth::LoginLimiter`): five failures are free, then the account is locked
for 30 seconds, doubling with each failure to at most 15 minutes, and a success clears the count. It is counted per
username rather than per address, since behind a reverse proxy every request comes from the proxy; the cost is that
someone who knows a username can keep that account locked by failing on purpose, which a household install accepts.
The counts are kept in memory, so restarting the server clears them.

`observe.rs` is how the server can be watched. `request_id`, the outermost layer, gives each request an id (a
proxy's `X-Request-Id` when it is plain, else a UUID) and returns it in the answer; `span` puts that id, the method
and the path (never the query, which can hold a kitchen link's code) on the span every log line of the request is
written in, as text or JSON (`MISE_LOG_FORMAT`). `measure`, inside routing, counts each request and times it under
the route it matched (`MatchedPath`), so `/api/v1/dishes/{slug}` is one series however many dishes are asked for;
`Metrics::render` writes the counts in Prometheus's text format for the metrics port (`MISE_METRICS_BIND`, served
by `metrics_router` beside the API, never on it). `/api/v1/ready` answers 503 (`unavailable`) when the database
doesn't answer within two seconds; `/api/v1/health` answers whenever the process does.

Recipes a household writes itself live under `/api/v1/recipes` (`recipes.rs`): create, read back as typed, replace,
delete, and set or remove the photo. A deleted recipe leaves every plan it was on, and its dish goes with it when
it was the dish's only version. The dish page's versions each say whether the household can edit them (`editable`). Only the household that wrote a recipe can read it there or change it; for anyone
else, and for a version that wasn't written in the app, the answer is the 404 of a recipe that doesn't exist. A
save is one transaction, and saves take turns under an advisory lock, so two recipes with one title never take the
same address. The recipe's dish follows its title: a new title gives it a new slug and retires the old one, which
still finds it. A photo is sent as the request's whole body with an image `Content-Type` (which a form on another
site can't set), read up to 20 MB on that route alone, checked from its header to be at most 50 megapixels before
anything is decoded, and decoded two at a time. `POST /api/v1/recipes/lines` reads lines without saving, for an
editor to show as the cook types, and `GET /api/v1/foods` (`foods.rs`) searches the foods to pick from. Foods are
shared by the whole install and matching makes one for any new ingredient name, so the search offers a household
only foods mise was given or a person edited, and matched foods that a version it can see uses: a name typed into
one household's private recipe is never offered to another.

The shopping list is never stored: `shopping::build` works it out from the week's plan on each request and lays the
stored marks, extras, staples and unit choices on top. Lines are grouped by food, and kinds of a food join its line
while a recipe in the week asks for the food itself (or the food is `shop_as_one`), never through a category; the
line is named for the one kind when the rest is the parent it covers. A food's section, count words and measures are
its own or its nearest ancestor's; pantry lines go under "Check you have". Marks are kept per food, so a line that
merges or splits keeps its ticks: a merged line is ticked when every food on it is, and a tick sent for a line marks
every food on the lines that now cover it.

## The web app (`web/`)

React 19 with React Router, Vite and TypeScript. `api/client.ts` is an `openapi-fetch` client typed from the
generated `api/schema.d.ts`; it acts on an error's `code`, never its status or wording (`unauthorized` signs the page
out, `account_changed` re-checks who is signed in). Pages load through TanStack Query (`api/query.ts`), which caches
answers per page, reloads them when a page comes back into view and keeps one account's data apart from another's.
`session.tsx` knows who is signed in (or, offline, who last was); `components/Layout.tsx` gates pages on it. Pages:
search, dish (options, servings, add to plan), recipe card (one version and serving count laid out to print on a
page), the recipe editor, plan, shopping, account, sign-in and first-run setup, and kitchen mode
(`pages/KitchenPage.tsx`: the week's dishes, then a dish a step at a time, with timers from
`components/Timers.tsx` and the screen kept awake), which is all a kitchen device opens. The editor
(`EditorPage.tsx`, with its rules in `editor.ts`) takes ingredients one to a line and steps one to a line, shows
how the server reads each ingredient line as the cook types (asked once typing pauses) with the food it counts
as, and lets the cook pick another; a pick belongs to the exact line it was made for, so changing the line's text
drops it. `kitchen.ts` splits a step round its times and finds the ingredients it names. `choose.ts` holds which version and serving count a
dish's address opens on, for the dish page and its card alike, and `recipe.ts` words a version's time, difficulty and
allergens for both.

Offline: `public/sw.js` keeps the app's files and recipe data (network first, cache as fallback; photos cache first)
and never caches account data. It reads the API's old and new addresses alike, so a page still running the app from
before the move to `/api/v1` keeps working through it. The shopping list keeps its own copy in `offline.ts`: each
week's list as last seen, plus ticks and hand-added items made without a connection, sent in order once the server
answers. Everything kept on the device is per account and cleared on sign-out: the list's copy, and the recipes and
photos the service worker kept, which it serves without asking the server and so also forgets when another household
signs in on the same browser. Service workers need HTTPS on a phone (or localhost).

## Deployment

`Dockerfile` builds one image with both binaries and the web app; `compose.yaml` runs it with Postgres and a nightly
`pg_dump` backup, and with the `https` profile, Caddy in front of it (`Caddyfile`). Settings are environment variables
with defaults (`deploy.env.example`); see [deploy.md](deploy.md). The server applies migrations when it starts.
