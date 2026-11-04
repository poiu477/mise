# Status and tasks

As of 2026-11-03. The design document holds the goals and the long-range roadmap; this file is the working list.
Update it in the same commit as the work.

## Where things stand

| Phase | State | What exists |
| --- | --- | --- |
| 1 Data core | Done; its gate (500 full-archive dishes checked by hand) was never recorded as passed, and the hand-trimmed launch seed took its place | Seed bundle format and importer; food matching, tree and curation files; dish grouping with drift splitting, protein twins and recorded decisions; options; content-addressed media with web-size copies; grouping gate report |
| 2 Browse and cook | Done (gate passed on 60 seed recipes, 2026-11-01) | REST API with OpenAPI; web app (PWA) with search, dish pages, option toggles, any serving count; step text rewritten for the chosen servings; local accounts, households, OIDC single sign-on, sessions, API tokens; Docker Compose stack with nightly backups |
| 3 Plan and shop | Core done (gate passed in use, 2026-11-03) | Weekly household plan; shopping list worked out from it, exact totals by store section, household and per-food weight units, pantry staples, hand-added items, ticks, printing, works offline on a phone |
| 4 Kitchen and imports | Not started | |
| 5 Optional extras | Not started | |

The live data is the launch seed: 237 dishes and 308 versions from meals received between October 2023 and October
2026, in database `mise_seed`.

## Next: repeatable gates and tests

Agreed direction (design document, "Tests and gates"): every gate becomes a command that passes or fails.

- [ ] `./check`: one command running clippy, all Rust and web tests, TypeScript, the web build, and a check that the
      generated API types match the server, and a git pre-commit hook that runs it (formatting: see below).
- [x] Demo bundle: `fixtures/demo-bundle`, four dishes (five versions) written for mise, covering two yields, a
      single yield, a pinch, a sized packet, pan-sauce water and a dish with two versions. Grow it as tests need.
- [ ] Pipeline test: import the demo bundle through every stage; check counts and a committed snapshot of every
      dish page at 1 to 6 servings.
- [ ] API tests for dishes, versions and photos (only sign-in, plan and shopping are covered today).
- [x] Browser journeys with Playwright against the demo bundle (`web/e2e/`, in CI): first-run setup; a dish at
      published and scaled counts; planning and the list in under 20 interactions with exact totals; ticks surviving
      a reload; ticking and adding offline, opening with no connection, and sending once back online; no sideways
      scroll at 375 px; an axe scan with no serious problems; sign-out leaving nothing on the device.
- [ ] `mise gate seed`: rebuild the private seed and check the invariants on every recipe at 1 to 6 servings, with a
      snapshot of rendered amounts kept in `seed/` and shown as a diff to approve. Replace the scaling report's
      "matches the source" score, which counts every round-up as a miss.
- [ ] Deploy gate: `docker compose up` on an empty volume, upgrade from the previous release's database, backup and
      restore.

## Production quality (the portfolio bar)

mise is a portfolio project, so it is held to production standards, not "good enough for one household". Recommended
order: the remote and CI baseline first (it makes everything after it checked), the gates above, then the rest.

- [x] GitHub repository (`poiu477/mise`, public since 2026-11-04) with CI on every push and pull request: formatting,
      clippy with warnings as errors, Rust tests against a Postgres service, an API-types check, web format, lint,
      tests and build, `cargo deny`, `npm audit` of shipped packages, the Docker image built. Dependabot weekly.
- [x] Standard formatting: rustfmt at its defaults and Prettier, each applied in one commit; ESLint with the React
      hooks rules.
- [x] Protect `main`: changes arrive through a pull request with CI passing; no force-pushes or deletion.
- [ ] Move page data loading to a data-fetching layer (TanStack Query or one shared hook), then turn ESLint's
      `react-hooks/set-state-in-effect` back from a warning (9 places) to an error.
- [x] Test single sign-on end to end: `crates/mise-api/tests/oidc.rs` runs a fake OpenID Connect provider that signs
      its own RS256, ES256 and EdDSA tokens, covering sign-up, return visits, admin by group, email linking, forged
      and mismatched tokens, replayed state and turned-off accounts.
- [ ] Share one test harness across `crates/mise-api/tests/` (auth, plan and oidc each set up a throwaway database
      and router the same way).
- [ ] Add Playwright to CI with the browser journeys, and push the Docker image to GitHub's registry on release
      tags.
- [ ] Database code a reviewer expects: `#[derive(sqlx::FromRow)]` structs instead of row tuples; compile-time checked
      queries with the `.sqlx` offline data committed so CI builds without a database; SQL moved out of the HTTP
      handlers into per-domain data modules.
- [ ] Architecture decision records in `docs/adr/` for the decisions below, with the alternatives weighed.
- [ ] Security: response headers (Content-Security-Policy, frame-ancestors, nosniff, referrer policy), the CSRF stance
      written down (SameSite=Lax cookies plus JSON-only bodies), `SECURITY.md`, dependency audits in CI. Login
      throttling exists.
- [ ] Observability: JSON logs as an option, a request id on every log line, a readiness check, basic metrics.
- [ ] API contract: decide on `/api/v1` and a standard error shape before anything outside the web app depends on it.
- [ ] Document every response each endpoint can return: the OpenAPI annotations list the main ones, but signed-in
      routes can all return 401 and 412 (`X-Mise-Account`), browser-only actions 403, and validation 400s are missing
      on some. Add a test that
      the documented statuses match what the handlers return.
- [x] Public-repo hygiene: test strings and examples written for mise; private paths and seed details moved to the
      ignored `CLAUDE.local.md`; the git history is published as it is (it never held recipe records or photos).
- [ ] Presentation: a README with screenshots and a short demo, a hosted demo running the demo bundle, versioned
      releases with a changelog.

## Phase 3, still to build

- [ ] Merge shopping lines at parent foods and across units with per-food densities (Onion and Yellow Onion are two
      lines today; "1 TBSP + 2 cloves" stays joined).
- [ ] Preferred products and stores per food; per-store aisle order.
- [ ] "Make from scratch": swap a proprietary item for its components.
- [ ] Nextcloud Tasks (list as CalDAV to-dos) and Calendar (plan as events or an iCal feed).
- [ ] Days in the plan (meals belong to a week today).

## Phase 4: kitchen and imports

- [ ] Kitchen mode for the Echo Show: one step at a time, large type, timers from step times, screen kept awake, a
      device token instead of a sign-in.
- [ ] Import from a URL (schema.org Recipe data) and from files (schema.org JSON, Mealie, Tandoor, Paprika, Nextcloud
      Cookbook).
- [ ] Manual editor and forks: our own versions of any dish.
- [ ] Review queues in the web app (they are CLI and report only today).
- [ ] Printable recipe cards.

## Housekeeping

- [ ] Rebuild the Docker image and run the full Compose stack on the seed; copy the `seed/` files the deploy steps name
      to the server's archive (use the `-seed` versions of curation, dish decisions and preferred versions).
- [ ] HTTPS in front of the server (through the reverse proxy), which phones need for offline start-up.
- [ ] A remote repository and off-machine backups of the database and `seed/`.

## Known issues and limits

- 36 versions (269 lines) publish 3-serving amounts equal to their 4-serving ones; left as published.
- Two review items stay open: green beans swapped for broccoli or carrots between versions; nothing records swaps yet.
- Some step text from the source lost a space in a mixed number ("10-12 11⁄2-inch meatballs").
- The pot pie dish's address still says "beef" (slugs are fixed at first import) until a fresh install.
- Scaled counts round up to halves (½ lime), not whole items; revisit if whole items read better in use.
- The fuzz suite takes about 25 seconds in a debug build, mostly the measure search.

## Open questions

- The Phase 3 browser-journey target: under 20 interactions from an empty week to a full list?
- Scaled counts: round up to halves (½ lime, today) or to whole items?
- HTTPS: which reverse proxy and hostname, so phones can open the list offline?
- Alexa route: a private skill through the reverse proxy, or an AWS Lambda relay (not before Phase 5)?

## Decisions

| Date | Decision |
| --- | --- |
| 2026-10 | Build our own app (Rust, Postgres, React) rather than adapt Mealie or Tandoor, which don't model dishes with versions |
| 2026-10 | No automated access to the seed's source and nothing of its content in the repo; the archive is a private, one-time seed |
| 2026-11-01 | Launch from the meals actually received (237 dishes, 308 versions); the full archive stays in `mise` |
| 2026-11-01 | A dish opens on the version had most, among those with steps |
| 2026-11-02 | Shopping totals are exact; weights read in the household's units, overridable per food |
| 2026-11-02 | Scaled recipe amounts round up, never down, into standard measures (one measure within 5%, else the fewest closest) |
| 2026-11-03 | The shopping list works offline from a device copy, with ticks and additions queued |
| 2026-11-03 | Gates become repeatable checks; demo data written for mise for public tests, the private seed for the seed gate |
| 2026-11-03 | mise is a portfolio project held to production standards; "only for one household" no longer justifies a shortcut |
| 2026-11-04 | The repository goes public with its history as it is; the private seed's details live in the ignored `CLAUDE.local.md` |
| 2026-11-04 | One pinned Rust (1.99.1) for local builds, CI and Docker, raised deliberately; `rust-version` matches it. RUSTSEC-2023-0071 (rsa, Marvin attack) is accepted in `deny.toml`: rsa only verifies sign-in providers' public-key signatures |
