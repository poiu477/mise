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

### T38 · Food swaps in the review queue

Grouping raises `food_swap` items (one food replaced by another between a dish's versions: green beans for broccoli),
but nothing records an answer, so the review page doesn't list them (status.md, Known issues).

- List `food_swap` items in the API's queue and the web page, with their dishes and versions. Answers: `alias` (two
  names for one food: a merge), `substitute` (a `food_relations` row of kind `substitute`, both ways), or `keep`
  (unrelated), stored so a rebuild keeps them and a kept pair isn't raised again.
- **Touches:** `crates/mise-api/src/review.rs`, `crates/mise-ingest/src/options.rs`, `web/src/pages/ReviewPage.tsx`,
  `docs/curation.md`.
- **Needs:** nothing.
- **Done when:** API tests answer a swap each way and see the relation (or the merge) and that a kept pair isn't
  raised again; a browser journey answers one from the review page; status.md's known issue about unlisted swaps is
  gone.

### T42 · Missing indexes and a unique dish address

From the code review. One migration:

- A unique constraint on `dishes.slug` (grouping keeps them unique today, but nothing in the database does).
- Indexes on the foreign keys deletes and lookups walk with none today: `plan_entries.variant_id` and the others a
  query of `pg_constraint` against `pg_index` lists.
- **Touches:** a new migration, `docs/architecture.md`.
- **Needs:** nothing in flight that adds a migration with the same number.
- **Done when:** a test asserts every foreign key column (from `pg_constraint`) has an index that starts with it, and
  inserting a second dish with a taken slug fails.

### T43 · Merging foods keeps what households chose

From the code review. A food merge and `mise match-foods --reset` silently cascade-delete households' pantry staples and
per-food weight units, and leave shopping ticks naming a food that is gone.

- Move pantry staples, weight-unit choices and shopping marks to the food merged into (keeping the household's choice
  when both had one), and keep them through a reset by moving them to the food the same name matches again.
- **Touches:** `crates/mise-ingest/src/curate.rs`, `foods.rs`.
- **Needs:** nothing.
- **Done when:** tests merge a food a household has as a staple, with a weight unit and a tick, and find all three on
  the food merged into; the same after `match-foods --reset`.

### T44 · Reading recipes: the parser's misreadings

From the code review, each reproduced against `mise-core` outside the repo. Each fix comes with a property test (the
crate already uses example-based tests; add `proptest` as a dev-dependency) or, where a property doesn't fit, a table
of cases written for mise.

- Spaced fractions in step text ("1 / 2 cup") read as 1 cup, even at the published yield.
- Compound number words ("thirty-five minutes") set a 5-minute timer.
- Linking a step's amount to a line matches prefixes: "peanut butter" links to the peas line.
- Brand words count as narrowing words: turkey bacon is filed as a kind of bacon.
- Black pepper is filed as a kind of pepper (the vegetable), ground ginger as a kind of ginger (the root).
- Plurals: cookies, pies and bay leaves don't fold to their singular, which makes duplicate foods.
- The protein option labels salmon steak as beef.
- Hyphenated mixed numbers ("1-1/2") read as 1.
- Quart, pint, stick and packet aren't units, so they stay in the food's name.
- Rounding disagrees with docs/scaling-rules.md: pounds round up in quarter-pound steps, and the relative slack lets
  large amounts round down. Fix whichever is wrong, the code or the doc, with the evidence.
- **Touches:** `crates/mise-core/src/`, `docs/scaling-rules.md`, `fixtures/demo-bundle/snapshots/` (if a demo reading
  changes).
- **Needs:** nothing.
- **Done when:** each item above has a failing test first, then passes; the pipeline snapshots change only where a
  reading was wrong, and the diff says which.

### T45 · Running it: shutdown, limits and the database role

From the code review's operations findings.

- Handle SIGTERM as well as Ctrl-C, so `docker stop` shuts down gracefully instead of being killed after 10 seconds.
- Settings for the connection pool's size and a statement timeout, with sensible defaults (today a fixed 8 and none).
- A limit on imports from a link running at once, as file imports have; and read an upload's body only once its turn
  comes, rather than holding 100 MB in memory while it waits.
- Connect as a role that owns mise's schema but isn't a superuser; `compose.yaml` makes it.
- First-run setup: the setup page is open to whoever reaches the server first on 0.0.0.0. Require a one-time setup
  code the server prints to its log at start while no account exists.
- `deploy.env.example`'s password advice can produce characters that break the database URL; use hex.
- Pin the images `compose.yaml` and CI use by digest, with Renovate keeping the digests current.
- **Touches:** `crates/mise-api/src/main.rs`, `lib.rs`, `imports.rs`, `auth/routes.rs`, `crates/mise-db/src/`,
  `compose.yaml`, `deploy.env.example`, `.gitlab-ci.yml`, `renovate.json`, `docs/deploy.md`.
- **Needs:** nothing in flight in `crates/mise-api/src/main.rs`.
- **Done when:** tests show a SIGTERM ends the server with its in-flight request answered; a second link import waits
  its turn; setup without the code is refused (and the browser journeys pass it); the `docker image` CI job runs the
  stack with the app's non-superuser role and the pool settings.

### T46 · Sign-in hardening

From the code review's medium findings in authentication.

- Sessions: renew the cookie (and the session's expiry) as it is used, with an absolute lifetime after which signing in
  is needed again.
- An admin's read-write API token can reset that admin's own password; keep password changes to sessions.
- The login throttle can be raced with parallel requests: count the attempt before checking the password.
- Hash passwords on a blocking thread everywhere (`/auth/setup` and the admin user routes do it on the runtime).
- Refetch the provider's signing keys when a token names a key id mise doesn't have, so key rotation doesn't break
  sign-in until a restart.
- **Touches:** `crates/mise-api/src/auth/`.
- **Needs:** nothing in flight in `crates/mise-api/src/auth/`.
- **Done when:** a test for each: a session used on day 29 still works on day 31, and none works past the absolute
  lifetime; a token's password change is refused; parallel bad logins lock the account at five; a provider that rotates
  its key mid-test still signs people in.

### T47 · Imports and pruning that fail cleanly

From the code review's data-integrity findings.

- A NUL character in imported text answers 500 and leaves the import half done: strip it (or refuse the file) before
  anything is stored.
- A failed import leaves recipes with no dish, and importing again answers 500: make the stages run in the import's
  transaction, or repair a dishless record on the next import.
- `mise prune` with an empty bundle deletes the whole source: refuse an empty bundle, and add `--dry-run`.
- Two people editing one recipe silently overwrite each other: send the version's `updated_at` with a change and answer
  409 when it is stale.
- **Touches:** `crates/mise-ingest/src/import.rs`, `files.rs`, `prune.rs`, `crates/mise-cli/src/main.rs`,
  `crates/mise-api/src/recipes.rs`, `web/src/pages/EditorPage.tsx`.
- **Needs:** T35 not in flight (both change `recipes.rs`).
- **Done when:** tests import a file and a page with NUL characters cleanly, import again after a failure midway,
  refuse `prune` with an empty bundle, and answer 409 to the second of two edits made from one copy.

### T48 · The web app on a bad connection

From the code review's frontend findings.

- Many mutations have no `.catch`, so a failed request leaves their button stuck; the plan's +/− taps race each other;
  the service worker's network-first fetch has no timeout, so the app can hang on a weak signal; the cache version is a
  constant edited by hand and cache storage grows without limit; error text fails WCAG contrast in dark mode; there is
  no app-level error page.
- **Touches:** `web/src/`, `web/public/sw.js`.
- **Needs:** T41 not in flight.
- **Done when:** browser journeys with requests failing (Playwright's `route.abort`) leave every button usable and say
  what failed; rapid taps on + land the right count; a stalled network falls back to the cache within a few seconds;
  axe passes in dark mode; a thrown render error shows the error page.

### T49 · Tests for dish grouping and the ingest modules

From the code review. Most of dish grouping (drift splits, protein swaps, twins across lineages) is reached only by the
private seed, and `prune.rs`, `curate.rs`, `foods.rs` and `bundle.rs` have no unit tests.

- Add bundle fixtures written for mise that make each grouping case happen, with pipeline snapshots, and unit tests
  for each of the four modules.
- **Touches:** `fixtures/`, `crates/mise-ingest/src/`, `crates/mise-cli/tests/`.
- **Needs:** nothing.
- **Done when:** removing any one grouping rule (drift split, protein swap, twin) fails a test that runs without the
  private seed; each of the four modules has tests for its main paths.

### T50 · Docs brought up to date

From the code review. README and AGENTS.md are behind status.md; ADRs 0001, 0002 and 0004 to 0007 describe choices
since changed; "only the health check is open" (deploy.md) is wrong; architecture.md leaves out the CLI's dependency on
`mise-api`. Also the review's small items: search totals past the last page, hero photos picked with `LIMIT 1` and no
`ORDER BY`, metrics labels growing from made-up HTTP methods, provider error text that can forge log lines, photo
decoding copying the image to check transparency.

- **Touches:** `README.md`, `AGENTS.md`, `docs/`, and for the small items `crates/mise-api/src/dishes.rs`,
  `observe.rs`, `auth/oidc.rs`, `crates/mise-ingest/src/media.rs`.
- **Needs:** nothing.
- **Done when:** each superseded ADR says what replaced it; a test for each small code item; the docs name no route,
  setting or file that no longer exists (a script in `./check` that checks the paths and settings the docs name).

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **Deploy gate:** a CI job that starts the Compose stack on an empty volume, upgrades a database from the previous
  release, and backs up and restores. Needs a released version to upgrade from (T10 makes them; the first is 0.1.0).
- **Presentation:** README screenshots (taken by Playwright from the demo bundle), a hosted demo (**Paul**: where),
  a short demo recording.
- **Scaled counts:** round up to halves (today) or to whole items.

## Only on Paul's machine

These need the private seed, which is not in the repo.

- **CI runners:** the Docker-socket runners (`docker-socket` tag) also take untagged jobs, so any job, a compromised
  dependency's build script included, gets the host's Docker and so root on the runner. Turn off **Run untagged jobs**
  on runners 6 and 8 (and tag the jobs that need Docker), so only `docker image` and `release image` reach the socket.
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

Phase 4, kitchen and imports, was broken into T22 to T33, all done: recipes owned by a household (T22), kitchen
devices and mode (T23, T24), import from a link and from files (T26, T27), writing recipes and the editor (T28,
T29), forks (T30), sharing between households (T31), review queues in the web app (T32) and printable cards (T33).
Its gate, T24's kitchen journey and T25's 20 sample pages, passes.

Break a phase item into Ready tasks, in the shape above, before starting it.
