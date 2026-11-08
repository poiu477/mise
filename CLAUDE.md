# Working on mise

mise is a self-hosted recipe system: dishes with versions, option toggles, any serving count, a shared weekly plan
and a shopping list that works offline. It runs as a Docker Compose stack in a Proxmox LXC and Paul's household uses
it daily, but it is a **portfolio project held to production standards**: code, tests, security, docs and process
should stand up to a reviewer's reading. "It's only for us" is never a reason to accept a shortcut. Phases 1 to 3
of the roadmap are done; see [docs/status.md](docs/status.md) for where things stand and
[docs/backlog.md](docs/backlog.md) for what's next.

Read these before changing anything:

| Doc | What it holds |
| --- | --- |
| [docs/status.md](docs/status.md) | What works, the engineering baseline, known issues, open questions, decisions |
| [docs/backlog.md](docs/backlog.md) | The work still to do, as self-contained tasks |
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

## Working as an agent

Work is split into tasks in [docs/backlog.md](docs/backlog.md) so several sessions can run at once.

1. **Take one task** from "Ready", whose "Needs" are met. Check `gh pr list` first: skip a task with a pull request
   open, or one that touches the same area as an open pull request.
2. **Branch from an up-to-date `main`**, named after the task (`t4-pipeline-snapshot`). When another session works
   in this checkout, use a separate git worktree.
3. **Do the task to its "Done when".** Add the tests it names. Stay inside "Touches"; if the work needs more, say so
   in the pull request rather than spreading quietly.
4. **Run the everyday commands below** and fix what they report.
5. **Update the docs in the same pull request:** delete the task from the backlog; record anything that changed in
   status.md (where things stand, a decision, a known issue); fix any doc the change makes wrong.
6. **Open a pull request** saying what changed, how it was checked, and anything left undone. CI, Mira (`@mira`) and
   CodeRabbit review it. Fix each valid finding, or reply with why it doesn't apply.
7. **Paul merges,** or says to merge once CI is green. Merge commits only (`gh pr merge --merge`).

Stop and ask instead of guessing when a task needs a decision listed under "Open questions", the private seed, a
change to GitHub settings, or anything outside the pull request (a release, a deploy, a message).

Running in parallel:

- **Migrations:** take the next free number when you open the pull request. If another merge took it first,
  renumber yours before merging. Never edit one that's on `main`.
- **Tests:** API tests each make their own throwaway database, so they can run side by side. The browser journeys
  use the fixed database `mise_e2e` and port 8095, so run them one session at a time (CI runs them for every pull
  request anyway).
- **Generated files:** after rebasing over another API change, regenerate `web/src/api/schema.d.ts` rather than
  merging it by hand.

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

- Postgres 16 runs in Docker (`compose.dev.yaml`) as `mise-db-1` on port 5433, user and password `mise`. The
  databases are described in [docs/development.md](docs/development.md).
- Machine-specific notes (paths, the private archive, which server runs where) live in `CLAUDE.local.md`, which git
  ignores. Read it when it exists.
