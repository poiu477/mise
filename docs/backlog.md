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

## Needs a decision first

Each of these waits on a question in status.md's "Open questions" or on Paul.

- **Database code:** `#[derive(sqlx::FromRow)]` structs, compile-time checked queries with `.sqlx` committed, SQL
  moved out of handlers into per-domain modules. It touches every file in `crates/mise-api/src/`, so it runs
  alone, with nothing else in flight there.
- **Deploy gate:** a CI job that starts the Compose stack on an empty volume, upgrades a database from the previous
  release, and backs up and restores. Needs a released version to upgrade from (T10 makes them; the first is 0.1.0).
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

Phase 4, kitchen and imports, was broken into T22 to T33, all done: recipes owned by a household (T22), kitchen
devices and mode (T23, T24), import from a link and from files (T26, T27), writing recipes and the editor (T28,
T29), forks (T30), sharing between households (T31), review queues in the web app (T32) and printable cards (T33).
Its gate, T24's kitchen journey and T25's 20 sample pages, passes.

Break a phase item into Ready tasks, in the shape above, before starting it.
