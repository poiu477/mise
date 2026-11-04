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

- **No third-party recipe content in the repo.** The household's own seed comes from a private archive kept for
  personal, non-commercial use: its recipes, photos, the tools that collected it, order history and anything derived
  from them stay out of git. Private material lives in the ignored `seed/` and `reports/` folders and outside the
  repo (see `CLAUDE.local.md`, which is ignored too). Tests and fixtures use text written for mise.
- **Stage files by name.** `git add <paths>`, never `git add -A` or `git add .`: an env file was committed that way
  once. Commit only when asked.
- **Work on a branch and merge through a pull request.** The repo is `poiu477/mise` on GitHub. CI (`.github/workflows/ci.yml`) must pass before merging; never push to `main`.
- **LF line endings.** sqlx checksums migrations, so `.gitattributes` pins LF. Scripts that rewrite files must write
  bytes (`Path.write_bytes(s.encode())` in Python), not `write_text`, which writes CRLF on Windows.
- **Never edit an applied migration.** Add a new numbered file in `migrations/`.
- **The dev database starts with no accounts** so the first visit is the owner's own setup. Anything a test or a
  check adds to `mise_seed` (accounts, plans, ticks) must be removed afterwards; use a far-off week such as
  2027-01-04 for trial plans.
- **Windows can't replace a running exe.** `cargo build --release` fails with "Access is denied" while
  `target/release/mise-server.exe` runs. Stop that server first, or use the debug build on another port: `MISE_BIND=127.0.0.1:8091 MISE_WEB_DIR=web/dist ./target/debug/mise-server.exe`.

## Everyday commands

What CI runs, so run it before pushing. `./check` runs it all in CI's order and names the step that fails:

```bash
./check                                # needs the dev Postgres (port 5433); API tests make throwaway databases
./check --e2e                          # and the browser journeys (one session at a time: fixed database and port)
./check --fast                         # formatting and lints only; the pre-commit hook (git config core.hooksPath .githooks)
cd web && npm run api                  # regenerate src/api/schema.d.ts after changing the API (needs a release build)
```

The steps it runs, for running one alone: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D
warnings`; `cargo test --workspace`; the API document check; then in `web/`, `npm run format:check`, `lint`, `test`,
`build` and (`--e2e`) `e2e`.

When the release build is locked, regenerate the types from the debug build instead:
`../target/debug/mise-server.exe openapi > openapi.json && npx openapi-typescript openapi.json -o src/api/schema.d.ts`.

## Environment

- Postgres 16 runs in Docker (`compose.dev.yaml`) as `mise-db-1` on port 5433, user and password `mise`. The
  databases are described in [docs/development.md](docs/development.md).
- Machine-specific notes (paths, the private archive, which server runs where) live in `CLAUDE.local.md`, which git
  ignores. Read it when it exists.
