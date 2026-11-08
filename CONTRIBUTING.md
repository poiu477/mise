# Contributing

Thanks for helping. mise is small and run by one maintainer, so work goes through a few fixed steps that let
several people, and their coding agents, work at once without colliding. The full rules are in
[AGENTS.md](AGENTS.md); this page is the short version.

mise lives on GitLab at <https://gitlab.pauldrakehughes.com/Personal/mise>. The GitHub repository is a read-only
mirror: issues and merge requests go to GitLab, and pull requests on GitHub aren't merged.

## Get it running

You need Rust (rustup installs the pinned version), Node 26, Docker and bash (Git Bash on Windows).
[docs/development.md](docs/development.md) sets up Postgres and a database of demo recipes, written for mise. The
household's own recipes are private and never in the repo, and you don't need them: every test and CI job runs on
the demo data.

## Pick something to do

[docs/backlog.md](docs/backlog.md) lists tasks under "Ready". Each says what it touches, what it needs first, and
the automated check that proves it's done. Pick one that:

- has no open merge request, and doesn't touch the same files as one;
- has its "Needs" met. Anything marked **Paul** waits on the maintainer.

Open a draft merge request, titled with the task number, as soon as you have a first commit: that's how a task is
claimed. Ideas that aren't in the backlog are welcome as issues first, so they can be shaped into a task.

## Make the change

- Fork the project on GitLab, branch from `main`, and keep to the files the task lists.
- Follow [docs/style.md](docs/style.md): how code, comments, commits and UI text read here.
- Run `./check` before pushing (`./check --e2e` to add the browser journeys), and fix what it reports.
- Update the docs in the same merge request: delete the task from the backlog, and note anything that changed in
  [docs/status.md](docs/status.md).

Coding agents are welcome. Point yours at [AGENTS.md](AGENTS.md) if it doesn't read it on its own, and review what it
writes as you would your own work.

## Review

Fill in the merge request template and mark the merge request ready. CI runs on it; a fork's pipeline starts once
the maintainer approves it. Fix each review finding, or reply with why it doesn't apply. The maintainer merges.

Contributions are under the project's [MIT licence](LICENSE).
