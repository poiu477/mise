# Mira's review rules for mise

Mira adds its per-repository rules to every review prompt. They're kept here so they change through merge requests
like everything else; paste the section below into Mira's dashboard (Rules, for `Personal/mise`) after a change
merges. `.mira.yaml` holds the settings (effort, thresholds, excluded files).

## Rules

mise is a self-hosted recipe app: a Rust workspace (axum, sqlx, utoipa) with Postgres, and a React and TypeScript web
app that works offline on a phone. Its maintainer's household uses it every day, and it is held to production
standards as a portfolio project, so judge changes as you would for a product with real users and real data. The
working rules are in `AGENTS.md`, and `docs/style.md` sets how code comments, docs and UI text are written; read
them with your file tools when a change touches what they govern.

CI already enforces formatting, clippy with warnings as errors, ESLint, Prettier, the test suites, browser
journeys, `cargo deny`, `npm audit`, GitLab's SAST and Semgrep. Spend your comments where a careful human reviewer
adds what those can't: whether the change is correct, whether its tests would catch it being wrong, and whether it
does what its merge request and backlog task say.

The places where mistakes here cost the most:

- Data belongs to a household. Every query that reads or changes plans, shopping lists, ticks, tokens or a
  household's own recipes must be scoped to the signed-in account's household. A missing or wrong scope leaks one
  household's data to another, so treat it as a blocker and name the query.
- Amounts are the product. Scaled amounts round up into standard measures and never shrink as servings grow
  (`docs/scaling-rules.md`); shopping totals add up exactly. A change to `mise-core`'s quantity, scaling, step or
  shopping code needs a test that pins the new behaviour.
- The shopping list queues changes made offline and sends them later, possibly after the plan or the signed-in
  account changed. Check that a queued change still lands on the right list, once, or is refused cleanly.
- A migration that is on `main` is never edited, because sqlx checksums it; new ones take the next free number.
- Only text written for mise goes into the repository. Flag recipe text, titles or photos that look copied from a
  real site or cookbook, in fixtures and tests too.
- A change that makes a doc wrong fixes the doc in the same merge request; a task's merge request deletes the task
  from `docs/backlog.md` and records what changed in `docs/status.md`.

Calibrate severity to consequence. A blocker is something that would break a user's data, their account's
isolation, or the build on `main`. Prefer one precise comment that names the failing input and what goes wrong
over several speculative ones; when you aren't sure, say what you would need to check rather than asserting it. If
the change is sound, say so briefly.
