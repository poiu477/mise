# Status

As of 2026-11-08. Where things stand, known issues, open questions and the decisions made. The work still to do is
in [backlog.md](backlog.md); the design document holds the goals and the long-range roadmap. Update both in the same
pull request as the work.

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

## Engineering baseline

What a reviewer can rely on today:

- **Repository:** public at `poiu477/mise` since 2026-11-04, history as it is. `main` is protected: changes arrive
  through a pull request with CI passing and the branch up to date; no force-pushes or deletion, admins included.
  Merges are merge commits, so the SHAs in `.git-blame-ignore-revs` stay valid.
- **CI** (`.github/workflows/ci.yml`, actions pinned to SHAs): rustfmt, clippy with warnings as errors, Rust tests
  against a Postgres service (API tests fail rather than skip when it's missing), the OpenAPI document checked and
  the web types compared with it, web format, lint, tests and build, `npm audit` of shipped packages, `cargo deny`,
  browser journeys, the Docker image built.
- **Local checks:** `./check` runs CI's build, lint and test steps with CI's commands, in CI's order, and names the
  step that fails; `--e2e` adds the browser journeys. `cargo deny`, `npm audit` and the image build stay CI-only. A
  pre-commit hook (`.githooks/`, opt-in) runs the formatting and lints.
- **Tests:** unit and fuzz tests in `mise-core`; API tests for sign-in, plan and shopping; single sign-on end to end
  against a fake OpenID Connect provider (RS256, ES256, EdDSA); Playwright journeys on the demo bundle covering
  first-run setup, scaling, planning to a full list, ticks across reloads, offline use, a 375 px layout, an axe scan
  and sign-out.
- **Demo data:** `fixtures/demo-bundle`, written for mise, under the MIT licence. The household's own seed is private
  and never committed.
- **Dependencies:** Dependabot weekly, grouped; TypeScript 7 held back until its tools support it, with a weekly
  workflow (`typescript-watch.yml`) that opens an issue when they do.
- **Reviews:** Mira (`@mira`) and CodeRabbit review every pull request.
- **Decisions:** architecture decision records in [`docs/adr/`](adr/README.md), each with the alternatives weighed.

## Known issues and limits

- 36 versions (269 lines) publish 3-serving amounts equal to their 4-serving ones; left as published.
- Two review items stay open: green beans swapped for broccoli or carrots between versions; nothing records swaps yet.
- Some step text from the source lost a space in a mixed number ("10-12 11⁄2-inch meatballs").
- The pot pie dish's address still says "beef" (slugs are fixed at first import) until a fresh install.
- Scaled counts round up to halves (½ lime), not whole items; revisit if whole items read better in use.
- The fuzz suite takes about 25 seconds in a debug build, mostly the measure search.
- An item added to the list offline can appear twice if the server saves it but its answer is lost: adding has no
  idempotency key, so the device sends it again.

## Open questions

- Scaled counts: round up to halves (½ lime, today) or to whole items?
- HTTPS: which reverse proxy and hostname, so phones can open the list offline?
- Alexa route: a private skill through the reverse proxy, or an AWS Lambda relay (not before Phase 5)?
- API contract: move to `/api/v1` with one error shape now, or when something outside the web app first uses it?
- Hosted demo: where does it run, and does it reset on a schedule?

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
| 2026-11-04 | Work moves to self-contained tasks in `docs/backlog.md`, one task per branch and pull request, so agent sessions can run in parallel |
| 2026-11-04 | The repository goes public with its history as it is; the private seed's details live in the ignored `CLAUDE.local.md` |
| 2026-11-04 | One pinned Rust (1.99.1) for local builds, CI and Docker, raised deliberately; `rust-version` matches it. RUSTSEC-2023-0071 (rsa, Marvin attack) is accepted in `deny.toml`: rsa only verifies sign-in providers' public-key signatures |
