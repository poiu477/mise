# Backlog

The work still to do, written so one agent session (or one person) can take a task and finish it without
other context. [status.md](status.md) says where things stand and why. How to take a task, claim it, branch, check
and open the merge request is in [AGENTS.md](../AGENTS.md) under "Taking a task".

Each task gives:

- **Touches:** the files or areas it changes. Two tasks that touch the same area shouldn't run at the same time.
- **Done when:** the check that proves it. Every check is automated; "looks right" is not a check.
- **Needs:** tasks to finish first, or a decision or action only Paul, the maintainer, can take (**Paul**).

Take tasks from **Ready** in the order listed, among those whose "Needs" are met: the order puts first what most
other tasks wait on. A task with an open merge request (a draft counts) is taken.
The merge request that finishes a task deletes it here and adds a line under "Where things stand" or "Decisions" in
status.md if it changed either.

## Ready

### T6 · Security headers and a written security stance

- Add response headers from the server: a Content-Security-Policy that the app and its service worker work under,
  `frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, a referrer policy.
- Write `SECURITY.md` (how to report a problem, what is in scope) and a short section in `docs/architecture.md` on
  the CSRF stance as the code enforces it: SameSite=Lax session cookies, JSON bodies where a route takes one, and
  what protects each state-changing route that takes no body (sign-out, the DELETE routes). Check each in the code;
  if one turns out unprotected, fix it (for example by requiring a custom header) rather than describe a protection
  that isn't there. Include login throttling.
- **Touches:** the router setup in `crates/mise-api/src/lib.rs`, `SECURITY.md`, `docs/architecture.md`.
- **Needs:** nothing else in flight in `crates/mise-api/src/lib.rs` (T9 edits it too).
- **Done when:** an API test checks each header on a page and an API response; an API test shows a cross-site
  style request (no JSON content type, no custom header) to sign-out and to a DELETE route is refused or harmless,
  as the written stance says; the browser journeys pass (they would catch a policy that breaks the app).

### T9 · Observability

- An option for JSON logs, a request id on every log line (and in the response), a readiness check that the
  database answers (next to the existing health check), and basic metrics (request counts and durations by route).
- **Touches:** `crates/mise-api/src/main.rs`, `lib.rs`, `docs/deploy.md`.
- **Needs:** nothing else in flight in `crates/mise-api/src/lib.rs` (T6 edits it too).
- **Done when:** API tests cover the readiness check and the request id header; deploy.md says how to turn JSON logs
  on.

### T10 · Release images

- On a `v*` tag, a release job in `.gitlab-ci.yml` builds the Docker image, pushes it to the GitLab project's
  container registry, and creates a GitLab release from `CHANGELOG.md` (start one, in Keep a Changelog form, with
  what exists today as 0.1.0).
- **Touches:** `.gitlab-ci.yml`, `CHANGELOG.md`, `docs/deploy.md` (pulling the image instead of building it).
- **Needs:** **Paul** to push the first tag, and the container registry turned on for the project.
- **Done when:** GitLab's CI lint accepts the file, and a test tag (`v0.1.0-rc.1`, pushed by Paul) publishes the
  image to the registry and creates the release; the test release and image are deleted afterwards. Syntax checks
  alone don't prove the release path.

### T16 · Rust doc comments: `mise-api`, then the lint for the whole workspace

- As `mise-core`, `mise-db` and `mise-ingest` already do, for `crates/mise-api` (213 items). In this crate `///` on a
  handler or a request or response type also becomes its description in the OpenAPI document, so regenerate
  `web/src/api/schema.d.ts` and keep `check-openapi.mjs` passing; write those comments for an API reader.
- Then move the lint to the workspace: `[workspace.lints.rust] missing_docs = "warn"` in `Cargo.toml` and
  `[lints] workspace = true` in every crate's `Cargo.toml`, removing the per-crate attributes. `mise-cli` is a binary
  and has no public items, so it's covered without work.
- **Touches:** `crates/mise-api/src/` (comments only), `web/src/api/schema.d.ts` (regenerated), every `Cargo.toml`.
- **Needs:** not while the database-code rework is in flight: it moves most of this crate.
- **Done when:** clippy passes workspace-wide with the lint in `Cargo.toml`; a missing doc comment in any crate fails
  it.

### T22 · Recipes that belong to a household

Imports, the editor and forks all make recipes that belong to one household; today every recipe is visible to every
account. The schema already has `user` and `web` sources and `variants.parent_variant_id` for lineage.

- Through a new migration, give each version an owning household (none for seed and bundle recipes, which stay
  visible to everyone) and a visibility (`private` by default; `partners` and `install` come with T31, so accept
  only `private` for now). A dish shows to a household when any version it can see belongs to it; the dish page,
  search, the plan and the shopping list only ever use versions the household can see.
- **Touches:** a new migration, `crates/mise-api/src/` (dishes, variants, plan, shopping queries),
  `crates/mise-ingest/src/dishes.rs` (grouping considers ownership: one household's version never joins a dish that
  only another household can see).
- **Done when:** API tests show a version owned by one household is invisible to another in search, the dish page,
  a direct fetch by id, and when added to a plan by id; the pipeline snapshot test still passes unchanged.

### T23 · Kitchen device tokens

The Echo Show's browser can't sign in comfortably or send headers, so kitchen mode runs on a device token.

- From the account page, a household admin issues a kitchen token with a name ("Kitchen Echo"); opening its one-time
  link on the device sets a long-lived cookie. A kitchen session reads the household's plan, dishes, versions and
  photos, and nothing else: no shopping list, accounts, names or emails, and no changes. Tokens are listed and
  revoked like API tokens.
- **Touches:** `crates/mise-api/src/auth/`, a new migration, `web/src/pages/AccountPage.tsx`.
- **Done when:** API tests show a kitchen session can read each allowed route and is refused every other one,
  that `/api/v1/me` reveals no personal data to it, and that a revoked token stops working at once.

### T24 · Kitchen mode

- A kitchen view of a planned dish for a 1280 × 800 screen: one step at a time in large type, the step's ingredients
  with amounts at the planned servings, swipe or tap to move, the screen kept awake (Screen Wake Lock API, which
  needs HTTPS: deploy.md section 7). A time in the step ("bake 10 to 12 minutes") is a button that starts a timer,
  with an alarm in the page; a range starts at its lower end and says when to check.
- Finding times in step text is pure logic: a function in `mise-core` with unit tests and doc examples.
- **Touches:** `crates/mise-core/src/` (step times), `web/src/` (a new page and route), `web/e2e/`.
- **Needs:** T23.
- **Done when:** this phase's gate, half one: a browser journey at 1280 × 800 signs in with a kitchen token, opens
  this week's first planned dish, steps through it, starts a timer from a step and sees it count down; the axe scan
  passes on the view.

### T26 · Import from a link

- `mise import url <url>` and `POST /api/v1/imports` with an "Import from a link" form in the web app. The server
  fetches the page and its photo (timeouts, a size limit, a redirect limit, and private and loopback addresses
  refused unless `IMPORT_ALLOW_PRIVATE=true`), parses it with T25, stores the raw record under the `web` source,
  and runs the shared stages (match foods, group dishes, derive options, parse steps). The recipe belongs to the
  importing household (T22). Importing the same unchanged page again changes nothing.
- **Touches:** `crates/mise-ingest/src/`, `crates/mise-cli/src/`, `crates/mise-api/src/` (a new module),
  `web/src/`, `docs/`.
- **Needs:** T22, T25.
- **Done when:** API tests import fixture pages served by a local test server, refuse a private address, and show a
  repeat import is a no-op; a browser journey imports a page and opens the new dish.

### T27 · Import from files

- Export files from other apps through the same records and shared stages as T26: schema.org JSON, Nextcloud
  Cookbook (a folder of `recipe.json` files), Mealie, Tandoor and Paprika (`.paprikarecipes`). `mise import file
  <path>` and an upload form in the web app; each format's fixture is written for mise.
- **Touches:** `crates/mise-ingest/src/` (a module per format), `crates/mise-cli/src/`, `crates/mise-api/src/`,
  `web/src/`, `fixtures/`.
- **Needs:** T22, T25.
- **Done when:** a test per format imports its fixture to the expected records; a browser journey uploads one file
  and opens the dish.

### T28 · Write a recipe: the API

- Create and update a household's own versions: title, yields, ingredient lines (each parsed with the line parser
  and matched to a food, with the match returned so the editor can show and correct it), steps with references to
  ingredients, and photos (uploaded, checked with the `image` crate, size-limited, stored like the others). Own
  versions use the `user` source and `origin = 'user'`, and belong to the household (T22).
- **Touches:** `crates/mise-api/src/` (a new module), `crates/mise-ingest/src/` (reusing the line parser and
  matcher), `web/src/api/schema.d.ts` (regenerated).
- **Needs:** T22.
- **Done when:** API tests create a recipe, edit it, add a photo, refuse a non-image and an oversize file, refuse
  another household's edit, and find the recipe in search and on its dish page.

### T29 · Write a recipe: the editor

- A page to write and edit a recipe on T28's API: ingredient lines typed as text, each showing the food it matched
  (and letting you pick another), steps, yields, photo upload; usable on a 375 px phone.
- **Touches:** `web/src/` (new pages), `web/e2e/`.
- **Needs:** T28.
- **Done when:** a browser journey writes a recipe from nothing, corrects one food match, adds a photo, saves, and
  opens it at another serving count; the axe scan and the 375 px check pass on the editor.

### T30 · Forks: your own version of any dish

- "Make my own version" on any version you can see copies it into the household (`parent_variant_id` keeps the
  lineage, and the dish page names the original and its author) and opens it in the editor. A fork joins the same
  dish as one of its versions.
- **Touches:** `crates/mise-api/src/`, `web/src/`.
- **Needs:** T29.
- **Done when:** API tests fork a seed version and one of the household's own, and show the original is unchanged
  and the fork belongs to the forking household; a browser journey forks, edits an amount, and sees both versions on
  the dish page.

### T31 · Sharing between households

As the design has it: each household sets a default visibility for the recipes and forks it makes, and any one can
override it.

- Levels `private`, `partners` (households this one has added as partners) and `install`; `SHARING_MAX_LEVEL` caps
  the most open level on the install (default `install`). Shared items are read-only to others, who can fork them;
  withdrawing a share hides the item but leaves forks already made. Plans, lists and preferences always stay
  private.
- **Touches:** a new migration, `crates/mise-api/src/`, `web/src/` (household settings, a visibility control on own
  recipes).
- **Needs:** T30.
- **Done when:** API tests cover each level, the partner link both ways, the cap, and a withdrawn share with an
  existing fork.

### T32 · Review queues in the web app

Food matches, food parents and doubtful dish merges wait in `review_items`, answered today only with `mise`
commands and the gate report.

- An admin page listing open items by kind, with the proposal and its confidence, to accept, reject or edit. A
  decision is stored the way the `mise` commands store it (food edits make the food `origin = user`; dish decisions
  go in `dish_decisions`), so a re-run keeps it; `mise food export` and `mise dish export` write them back out as
  curation files.
- **Touches:** `crates/mise-api/src/` (a new module), `crates/mise-cli/src/` (the exports), `web/src/`.
- **Done when:** API tests decide one item of each kind and show a re-run of the stage keeps the decision; the
  exports round-trip through `apply`; a browser journey accepts a food match.

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **Database code:** `#[derive(sqlx::FromRow)]` structs, compile-time checked queries with `.sqlx` committed, SQL
  moved out of handlers into per-domain modules. It touches every file in `crates/mise-api/src/`, so it runs
  alone, with nothing else in flight there.
- **Deploy gate:** a CI job that starts the Compose stack on an empty volume, upgrades a database from the previous
  release, and backs up and restores. Needs T10 (a previous release to upgrade from).
- **Presentation:** README screenshots (taken by Playwright from the demo bundle), a hosted demo (**Paul**: where),
  a short demo recording.
- **Scaled counts:** round up to halves (today) or to whole items.

## Only on Paul's machine

These need the private seed, which is not in the repo.

- **`mise gate seed`:** rebuild the private seed and check the invariants on every recipe at 1 to 6 servings, with a
  snapshot of rendered amounts kept in `seed/` and shown as a diff to approve. Include Phase 1's gate in its
  repeatable form: the rebuild gives the same dishes and groupings (counts, and a fingerprint of which versions share
  a dish), with no open review items outside an allowlist. Replace the scaling report's
  "matches the source" score, which counts every round-up as a miss.
- **Deploy the stack on the seed:** rebuild the image, run the full Compose stack, copy the `seed/` files deploy.md
  names to the server (the `-seed` versions of curation, dish decisions and preferred versions).
- **HTTPS for the household's install:** proxy mise through the existing Caddy, as deploy.md section 7 says.
- **Dish addresses on the seed:** run `group-dishes` (then `derive-options`) on the seed once T21's migration is in,
  so renamed dishes such as the pot pie take addresses that match their names; then drop that known issue from
  status.md.
- **Off-machine backups** of the database and `seed/`.
- **GitHub mirror settings:** turn on secret scanning (free on public repositories; GitLab's pipeline already runs
  secret detection).

## Later phases

Phase 3, still to build:

- Preferred products and stores per food; per-store aisle order.
- "Make from scratch": swap a proprietary item for its components.
- Nextcloud Tasks (the list as CalDAV to-dos) and Calendar (the plan as events or an iCal feed).
- Days in the plan (meals belong to a week today).

Phase 4, kitchen and imports, is broken into T22 to T32 under Ready: recipes owned by a household, kitchen
mode, import from a link and from files, the editor and forks, sharing, and review queues in the web app (printable
cards, T33, are done). Its gate is T24's kitchen journey and T25's 20 sample pages (that half passes since T25).

Break a phase item into Ready tasks, in the shape above, before starting it.
