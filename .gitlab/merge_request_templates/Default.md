<!-- Title: the backlog task number and name, e.g. "T4: Pipeline snapshot test". Open as a draft to claim the task. -->

**Task:** <!-- T-number from docs/backlog.md, or the issue this closes (Closes #N) -->

## What changed

## How it was checked

<!-- The task's "Done when", and what you ran: ./check, ./check --e2e, new tests. -->

## Left undone

<!-- Anything outside the task's "Touches" that it needed, follow-ups, or "nothing". -->

- [ ] `origin/main` is merged into the branch (conflicts in the docs resolved keeping both sides)
- [ ] `./check` passes (and `./check --e2e` if the web app or API changed)
- [ ] The task is deleted from `docs/backlog.md`, and `docs/status.md` records anything that changed
- [ ] No recipe content that wasn't written for mise
