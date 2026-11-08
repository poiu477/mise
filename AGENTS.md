# Working on mise

These are the working rules for anyone changing mise: a person, or an agent in any coding tool (Claude Code,
Codex, Copilot, Cursor and others read this file). `CLAUDE.md` imports it, so the rules live here once.

mise is a self-hosted recipe system: dishes with versions, option toggles, any serving count, a shared weekly plan
and a shopping list that works offline. It runs as a Docker Compose stack, and its maintainer's household uses it
daily, but it is a **portfolio project held to production standards**: code, tests, security, docs and process
should stand up to a reviewer's reading. "It's only for one household" is never a reason to accept a shortcut.
Phases 1 to 3 of the roadmap are done; see [docs/status.md](docs/status.md) for where things stand and
[docs/backlog.md](docs/backlog.md) for what's next.

Read these before changing anything:

| Doc | What it holds |
| --- | --- |
| [docs/status.md](docs/status.md) | What works, the engineering baseline, known issues, open questions, decisions |
| [docs/backlog.md](docs/backlog.md) | The work still to do, as self-contained tasks |
| [docs/architecture.md](docs/architecture.md) | How the crates, database, API and web app fit together |
| [docs/development.md](docs/development.md) | Setup, databases, running, testing |
| [docs/style.md](docs/style.md) | How code, comments, commits and UI text are written here |
| [docs/adr/](docs/adr/README.md) | Why the big choices were made, with the alternatives weighed |
| [docs/scaling-rules.md](docs/scaling-rules.md) | How amounts scale and round, with the evidence |
| [docs/curation.md](docs/curation.md) | Food curation, dish decisions, preferred versions |

Paul Drake Hughes (`paulh` on GitLab, `poiu477` on GitHub) maintains mise; "Paul" in the docs means him. He also keeps a
design document with the long-range goals, which isn't public. Nothing a task needs lives only there: the backlog,
status and the decision records hold it.

## You don't need the private seed

The household's own recipes come from a private archive that is never in the repo. Everything else runs on the demo
recipes in `fixtures/demo-bundle`, written for mise: development, every test, CI and the browser journeys.
[docs/development.md](docs/development.md) sets up a `mise_demo` database from them. Tasks that need the private seed
are listed apart in the backlog ("Only on Paul's machine"); leave those to him.

## Rules that are easy to break

- **No third-party recipe content in the repo.** No recipe text, titles, photos or anything derived from them that
  wasn't written for mise. Tests and fixtures use text written for mise. Private material lives in the ignored
  `seed/` and `reports/` folders and outside the repo.
- **Stage files by name.** `git add <paths>`, never `git add -A` or `git add .`: an env file was committed that way
  once. Agents commit only when asked.
- **Work on a branch and merge through a merge request.** The repository lives on GitLab at
  <https://gitlab.pauldrakehughes.com/Personal/mise>, where `main` is protected and CI (`.gitlab-ci.yml`) must pass
  before merging. GitHub (`poiu477/mise`) is a read-only mirror of every branch: never push or open pull requests
  there.
- **LF line endings.** sqlx checksums migrations, so `.gitattributes` pins LF. Scripts that rewrite files must write
  bytes (`Path.write_bytes(s.encode())` in Python), not `write_text`, which writes CRLF on Windows.
- **Never edit a migration that's on `main`.** Add a new numbered file in `migrations/`.
- **A new database starts with no accounts** so the first visit is the owner's own setup. Anything a test or a check
  adds to a development database (accounts, plans, ticks) must be removed afterwards; use a far-off week such as
  2027-01-04 for trial plans.
- **On Windows, a running exe can't be replaced.** `cargo build --release` fails with "Access is denied" while
  `target/release/mise-server.exe` runs. Stop that server first, or use the debug build on another port:
  `MISE_BIND=127.0.0.1:8091 MISE_WEB_DIR=web/dist ./target/debug/mise-server.exe`.

## Taking a task

Work is split into tasks in [docs/backlog.md](docs/backlog.md), so several people and agent sessions can work at
once without stepping on each other.

1. **Take one task** from "Ready" whose "Needs" are met. Check the open merge requests first (`glab mr list`, or
   the project's Merge requests page): skip a task that has one open, and one whose "Touches" overlaps an open merge
   request's.
2. **Branch from an up-to-date `main`**, named after the task (`t4-pipeline-snapshot`). Without write access, fork
   the project on GitLab and work from your fork. When another session works in the same checkout, use a separate
   git worktree.
3. **Claim it** by opening a draft merge request as soon as you have a first commit, titled with the task number
   (`T4: Pipeline snapshot test`). An open merge request is the claim; there is no other list to update. Pushing
   can open it: `git push -o merge_request.create -o merge_request.draft -o merge_request.title="T4: Pipeline
   snapshot test" origin t4-pipeline-snapshot`.
4. **Do the task to its "Done when".** Add the tests it names. Stay inside "Touches"; if the work needs more, say so
   in the merge request rather than spreading quietly.
5. **Run the everyday commands below** and fix what they report.
6. **Update the docs in the same merge request:** delete the task from the backlog; record anything that changed in
   status.md (where things stand, a decision, a known issue); fix any doc the change makes wrong.
7. **Mark the merge request ready** once it's done, filling in the template: what changed, how it was checked,
   anything left undone. CI runs on it (a fork's pipeline waits for Paul to start it), and Mira (`@mira`), the
   project's AI reviewer, reviews it. Fix each valid finding, or reply with why it doesn't apply.
8. **Paul merges,** with a merge commit (the project merges no other way), or says to merge once the pipeline
   passes.

Stop and ask in the merge request (or an issue) instead of guessing when a task needs a decision listed under "Open
questions", the private seed, a change to GitLab or GitHub settings, or anything outside the merge request (a
release, a deploy, a message).

Running in parallel:

- **Migrations:** take the next free number when you open the merge request. If another merge took it first,
  renumber yours before merging. Never edit one that's on `main`.
- **Tests:** API tests each make their own throwaway database, so they can run side by side. The browser journeys
  use the fixed database `mise_e2e` and port 8095, so run them one session at a time on a machine (CI runs them for
  every merge request anyway).
- **Generated files:** after rebasing over another API change, regenerate `web/src/api/schema.d.ts` rather than
  merging it by hand.

## Everyday commands

Run these before pushing. `./check` runs CI's build, lint and test steps in CI's order and names the step that fails.
It isn't the whole CI gate: the browser journeys need `--e2e`, and `cargo deny`, `npm audit` and the Docker image
build run only in CI. It's a bash script; on Windows, run it from Git Bash.

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
`../target/debug/mise-server openapi > openapi.json && npx openapi-typescript openapi.json -o src/api/schema.d.ts`.

## Environment

- Rust 1.99.1 (pinned in `rust-toolchain.toml`), Node 26, Docker, and bash (Git Bash on Windows).
- Postgres 16 runs in Docker (`compose.dev.yaml`) on port 5433, user and password `mise`. The databases are
  described in [docs/development.md](docs/development.md).
