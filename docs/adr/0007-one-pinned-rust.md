# 0007 · One pinned Rust for local builds, CI and Docker

Accepted, 2026-11-04 (status.md's Decisions; commits "Pin Rust 1.99.1 and clear the dependency advisories" and
"Keep one Rust everywhere and close review gaps", the second from Mira's review).

## Context

CI runs clippy with warnings as errors and rustfmt's check. Both change with the compiler: a new stable Rust can
add lints, so an untouched branch can fail CI the day a release comes out. CI's first run also found dependency
advisories whose fixes needed a newer Rust (`time` 0.3.47 needs 1.88), so the workspace's minimum had to rise
anyway. Before this, local builds, CI and the Docker image could each use a different compiler.

## Decision

[rust-toolchain.toml](../../rust-toolchain.toml) pins one Rust (1.99.1, with rustfmt and clippy), and everything
builds with it:

- **Local builds and CI's Rust jobs** read the file through rustup (`rustup toolchain install` in the Rust and
  browser-journey jobs).
- **Docker** uses the same `rust:1.99.1` tag and copies `rust-toolchain.toml` into the build, so rustup enforces the
  pin there too ([Dockerfile](../../Dockerfile)).
- **CI's cargo-deny step** doesn't build, so it names the same version itself.
- **`rust-version`** in [Cargo.toml](../../Cargo.toml) is the declared minimum, `1.99`: it doesn't pin, but it is
  kept at the pinned release and raised with it.
- **Raised by hand**, all four in one commit. Renovate ignores the rust images
  ([renovate.json](../../renovate.json); Dependabot did the same until 2026-11-08), so they can't move on their
  own.

## Alternatives

- **Follow stable.** No upkeep, but CI results change with the calendar rather than the code, and a release with a
  new lint fails every open pull request at once.
- **A minimum version only (`rust-version`) and stable everywhere.** It says what the code needs, but not what
  builds it; local, CI and the image can still differ and disagree about lints.
- **Let Dependabot bump the rust image.** It would move the Docker build ahead of the toolchain file and CI, which
  is the split this decision removes.

## Consequences

- A build, a lint and a format check give the same answer on a laptop, in CI and in the image.
- Upgrading Rust is a deliberate change of four places, which the comment in `rust-toolchain.toml` lists. New
  language features (let-chains, used since the pin) arrive only when someone raises it.
- A dependency fix that needs a newer compiler waits for that change, as `time`'s did. cargo-deny in CI reports
  such advisories ([deny.toml](../../deny.toml)), so the wait is visible rather than silent.
