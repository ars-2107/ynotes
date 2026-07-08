# Contributing to ynotes

Thank you for considering a contribution. This document is short on purpose:
follow it and your change will pass review on the first pass.

## Ground rules

1. **Behaviour lives in the engine crate** (`crates/ynotes-core`, whose lib
   target is named `ynotes`, so `use ynotes::…` holds everywhere). The
   front-end crates — `crates/ynotes-cli` (the binary, with its `cli`,
   `commands`, `logging`, `command_error` modules) and `crates/ynotes-mcp` (the
   MCP server, the sole home of async) — may parse arguments, speak a protocol,
   and render results — nothing more. If a front-end would need the logic, it
   belongs in `ynotes-core`. The crate boundary makes this compiler-enforced;
   do not erode it.
2. **No `unsafe`.** The crate forbids it. ynotes has no need for it.
3. **Never silently drop user data.** Context a user wrote is sacrosanct. A
   failure to anchor it must surface flagged, never discard it. (See the
   design discussion in `AGENTS.md`.)

## Workflow

```sh
just            # list tasks
just ci         # fmt (rust+toml) + clippy + nextest/doctests + cargo-deny
```

`just ci` is exactly what CI runs. Green locally ⇒ green in CI. Run it before
opening a pull request. Tests run under `cargo nextest`; doctests run via
`cargo test --doc` (nextest does not execute doctests). Snapshot tests use
[`insta`](https://insta.rs) — after intentionally changing rendered output,
run `cargo insta review` and commit the updated snapshots.

### Commits

Use [Conventional Commits](https://www.conventionalcommits.org/): `feat:`,
`fix:`, `docs:`, `refactor:`, `test:`, `chore:`. Imperative mood, subject under
~72 characters.

### Minimum Supported Rust Version

MSRV is **1.85**, CI-enforced, declared in three places that must stay in sync:
`rust-version` in `Cargo.toml`, `msrv` in `clippy.toml`, and the `msrv` job in
`.github/workflows/ci.yml`. Raising it is a deliberate, documented decision
(note it in `CHANGELOG.md`).

## The comment doctrine

Comments are reviewed as carefully as code. The standard, from the official
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/):

- **Module (`//!`)** — every module opens with what it is responsible for and
  any invariant it upholds. *Why it exists*, not a list of its items.
- **Public items (`///`)** — every public item documented. `Result` returners
  carry `# Errors`; functions that can panic carry `# Panics`; non-trivial
  APIs carry `# Examples` (compiled as doctests, so they cannot rot).
- **Inline (`//`)** — only for the non-obvious *why*. Never restate the code.
  `// increment i` is deleted in review; `// clap returns 2 for usage errors,
  which this mirrors` is kept.
- **Doc links** — link types with `[`Type`]`. Broken intra-doc links are a
  hard CI error.

A useful test: if deleting the comment loses no information a competent Rust
reader couldn't recover from the code in five seconds, delete it. If it encodes
a decision, a constraint, or a reference, keep it.
