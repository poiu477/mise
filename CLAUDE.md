# Working on mise

mise is a self-hosted recipe system: dishes with versions, option toggles, any serving count, a shared weekly plan
and a shopping list that works offline. It runs as a Docker Compose stack in a Proxmox LXC and Paul's household uses
it daily, but it is a **portfolio project held to production standards**: code, tests, security, docs and process
should stand up to a reviewer's reading. "It's only for us" is never a reason to accept a shortcut. Phases 1 to 3
of the roadmap are done; see [docs/status.md](docs/status.md) for where things stand and what's next.

Read these before changing anything:

| Doc | What it holds |
| --- | --- |
| [docs/status.md](docs/status.md) | What works, the task list, known issues, decisions made |
| [docs/architecture.md](docs/architecture.md) | How the crates, database, API and web app fit together |
| [docs/development.md](docs/development.md) | Setup, databases, rebuilding the seed, running, testing |
| [docs/style.md](docs/style.md) | How code, comments, commits and UI text are written here |
| [docs/scaling-rules.md](docs/scaling-rules.md) | How amounts scale and round, with the evidence |
| [docs/curation.md](docs/curation.md) | Food curation, dish decisions, preferred versions |
| Design document | Goals, design and roadmap: <https://claude.ai/artifact/4LznYY3aaFkRnEsbQwp3D3> (a Claude Docs doc, id `1b1628b6-0190-46d3-847c-d3b64188359a`) |

## Rules that are easy to break

- **No HelloFresh content in the repo.** Recipes, photos, the scraper, order history and anything derived from them
  stay out of git (HelloFresh Terms of Use §16.8; personal, non-commercial use only). Private material lives in the
  ignored `seed/` and `reports/` folders and in `D:/Recipes` and `../hellofresh-backup/`. Tests use made-up data.
- **Stage files by name.** `git add <paths>`, never `git add -A` or `git add .`: an env file was committed that way
  once. Commit only when asked.
- **Work on a branch and merge through a pull request.** The repo is `poiu477/mise` on GitHub (private until the
  public-repo hygiene task is done). CI (`.github/workflows/ci.yml`) must pass before merging; never push to `main`.
- **LF line endings.** sqlx checksums migrations, so `.gitattributes` pins LF. Scripts that rewrite files must write
  bytes (`Path.write_bytes(s.encode())` in Python), not `write_text`, which writes CRLF on Windows.
- **Never edit an applied migration.** Add a new numbered file in `migrations/`.
- **The dev database starts with no accounts** so the first visit is the owner's own setup. Anything a test or a
  check adds to `mise_seed` (accounts, plans, ticks) must be removed afterwards; use a far-off week such as
  2027-01-04 for trial plans.
- **Windows can't replace a running exe.** `cargo build --release` fails with "Access is denied" while
  `target/release/mise-server.exe` runs. Ask Paul to stop his server (it usually runs in his Terminal 1) or use the
  debug build on another port: `MISE_BIND=127.0.0.1:8091 MISE_WEB_DIR=web/dist ./target/debug/mise-server.exe`.

## Everyday commands

What CI runs, so run it before pushing:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # needs the dev Postgres (port 5433); API tests make throwaway databases
cd web && npm run format:check && npm run lint && npm test && npm run build
cd web && npm run e2e                  # browser journeys; needs `cargo build -p mise-cli -p mise-api` and the build above
cd web && npm run api                  # regenerate src/api/schema.d.ts after changing the API (needs a release build)
```

When the release build is locked, regenerate the types from the debug build instead:
`../target/debug/mise-server.exe openapi > openapi.json && npx openapi-typescript openapi.json -o src/api/schema.d.ts`.

## Environment

- Repo: `H:\Dev\Projects\Claude Code\mise` (Windows 11; Git Bash and PowerShell). Postgres 16 runs in Docker as
  `mise-db-1` on port 5433, user and password `mise`.
- Databases: `mise_seed` is the live one (237 dishes, 308 versions from meals the household received); `.env` points
  at it with `MEDIA_ROOT=./data/media-seed`. `mise` holds the full archive (about 9,900 dishes) as evidence for
  package sizes; `seed/env-full-archive` has its settings.
- Paul runs `./target/release/mise-server` on port 8080 and uses it day to day.
