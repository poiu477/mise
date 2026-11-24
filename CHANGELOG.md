# Changelog

What changes in each release of mise, newest first, in the form of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html); until 1.0.0 a minor version may change
the API or the settings, and says how here.

A release is a `v` tag on `main` (`v0.1.0`). Its pipeline publishes the Docker image to the project's container
registry and makes a GitLab release whose notes are the version's section below; docs/development.md says how to cut
one.

## [Unreleased]

## [0.1.0] - 2026-11-24

The first release: everything mise does today.

### Added

- **Recipes as dishes with versions.** A seed bundle importer, food matching against a food tree with curation files,
  dish grouping (drift splitting, protein twins, recorded decisions), options that pick between a dish's versions,
  and photos stored once by content with web-size copies.
- **Cooking at any serving count.** Amounts scale and round as a cook would, step text is rewritten for the chosen
  yield, and a version reads as published at its published yields. Printable recipe cards at any serving count.
- **A REST API** under `/api/v1`, described by an OpenAPI document that lists every status each route answers, with
  one error body (`error` and a stable `code`).
- **The web app**, an installable PWA: search, dish pages with option toggles, the household's weekly plan, and a
  shopping list worked out from it (exact totals by store section, kinds of one food on one line, weight units,
  pantry staples, hand-added items, ticks), which works offline on a phone.
- **Households and accounts.** Password sign-in and single sign-on (OpenID Connect), admins, API tokens (read or
  read-write), and kitchen devices that read the plan and recipes through a one-time link.
- **Kitchen mode** for a 1280 × 800 screen: one step at a time in large type, the ingredients each step names,
  timers with an alarm, and the screen kept awake.
- **Recipes a household writes, forks and shares.** An editor that reads ingredient lines as they are typed; "Make my
  own version" of any recipe; sharing with partner households or the whole install, capped by `SHARING_MAX_LEVEL`.
- **Imports** from a link (the server fetches the page from the public internet only) and from the files other recipe
  apps export: schema.org JSON, Nextcloud Cookbook, Mealie, Tandoor and Paprika.
- **Review queues** for what importing leaves an admin to decide: food matches, food parents and dish merges.
- **Running it:** a Docker Compose stack with Postgres, daily backups and an optional Caddy for HTTPS; security
  headers and refusal of requests other sites send; JSON logs, request ids, a readiness check and Prometheus metrics
  on a port of their own.

[Unreleased]: https://gitlab.pauldrakehughes.com/Personal/mise/-/compare/v0.1.0...main
[0.1.0]: https://gitlab.pauldrakehughes.com/Personal/mise/-/releases/v0.1.0
