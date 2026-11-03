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
- [ ] Demo bundle: a dozen recipes we write ourselves (no HelloFresh content), in the repo, covering two yields,
      pinches, packets, pan sauces and a dish with two versions.
- [ ] Pipeline test: import the demo bundle through every stage; check counts and a committed snapshot of every
      dish page at 1 to 6 servings.
- [ ] API tests for dishes, versions and photos (only sign-in, plan and shopping are covered today).
- [ ] Browser journeys with Playwright against the demo bundle: first-run setup; search, dish, servings; plan four
      dishes and reach the list in under 20 interactions; tick offline, reconnect, saved once; sign-out clears the
      device; no sideways scroll at 375 px; an accessibility scan.
- [ ] `mise gate seed`: rebuild the private seed and check the invariants on every recipe at 1 to 6 servings, with a
      snapshot of rendered amounts kept in `seed/` and shown as a diff to approve. Replace the scaling report's
      "matches HelloFresh" score, which counts every round-up as a miss.
- [ ] Deploy gate: `docker compose up` on an empty volume, upgrade from the previous release's database, backup and
      restore.

## Production quality (the portfolio bar)

mise is a portfolio project, so it is held to production standards, not "good enough for one household". Recommended
order: the remote and CI baseline first (it makes everything after it checked), the gates above, then the rest.

- [ ] A public GitHub repository with CI on every push: formatting, clippy with warnings as errors, Rust tests against
      a Postgres service, web lint and tests, Playwright, `cargo deny` and `npm audit`, the Docker image built and
      pushed. Work on branches and merge through pull requests.
- [ ] Standard formatting: rustfmt at its defaults in one commit; ESLint and Prettier for the web app.
- [ ] Database code a reviewer expects: `#[derive(sqlx::FromRow)]` structs instead of row tuples; compile-time checked
      queries with the `.sqlx` offline data committed so CI builds without a database; SQL moved out of the HTTP
      handlers into per-domain data modules.
- [ ] Architecture decision records in `docs/adr/` for the decisions below, with the alternatives weighed.
- [ ] Security: response headers (Content-Security-Policy, frame-ancestors, nosniff, referrer policy), the CSRF stance
      written down (SameSite=Lax cookies plus JSON-only bodies), `SECURITY.md`, dependency audits in CI. Login
      throttling exists.
- [ ] Observability: JSON logs as an option, a request id on every log line, a readiness check, basic metrics.
- [ ] API contract: decide on `/api/v1` and a standard error shape before anything outside the web app depends on it.
- [ ] Public-repo hygiene: replace the HelloFresh step text quoted in tests (about 23 lines) with our own; decide
      whether to publish the git history as it is or start the public repo fresh.
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
- [ ] LICENSE holder name: "Paul Hughes" or "Paul D Hughes".

## Known issues and limits

- Opening the app from cold with no connection is untested; the built-in browser pane can't run the service worker.
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
| 2026-10 | No automated HelloFresh access and nothing of theirs in the repo; the archive is a private, one-time seed |
| 2026-11-01 | Launch from the meals actually received (237 dishes, 308 versions); the full archive stays in `mise` |
| 2026-11-01 | A dish opens on the version had most, among those with steps |
| 2026-11-02 | Shopping totals are exact; weights read in the household's units, overridable per food |
| 2026-11-02 | Scaled recipe amounts round up, never down, into standard measures (one measure within 5%, else the fewest closest) |
| 2026-11-03 | The shopping list works offline from a device copy, with ticks and additions queued |
| 2026-11-03 | Gates become repeatable checks; HelloFresh-free demo data for public tests, the private seed for the seed gate |
| 2026-11-03 | mise is a portfolio project held to production standards; "only for one household" no longer justifies a shortcut |
