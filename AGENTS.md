# AGENTS.md

Instructions for any agent or contributor working in this repository. Read this
before changing code — it encodes decisions that are expensive to reverse.

## What ynotes is

A git-like CLI that stores context for regions of code and returns, on demand,
only the context overlapping a queried line range. Not a comment system
(context costs no tokens until asked for) and not a notes vault (context is
anchored to the code and re-anchors when the code moves). The defining hard
problem — re-anchoring across edits with explicit staleness rather than silent
loss — is unsolved in this scaffold *by design*.

## Commands

```sh
just                 # list every task
just ci              # the gate: fmt(rust+toml) · clippy -D warnings · tests · deny
cargo run -- doctor  # run the binary
cargo nextest run --all-features   # tests (doctests: cargo test --doc)
cargo clippy --all-targets --all-features -- -D warnings
cargo insta review   # accept changed snapshots after intentional output changes
```

`just ci` is exactly what CI enforces. Green locally ⇒ green in CI. Nothing is
merged or committed without it passing.

## Architecture (and the one rule)

One crate, two faces (bat's model):

```
src/lib.rs          the engine — every behaviour. THIS is the library.
src/error.rs        the engine's typed error (a library module).
src/main.rs         a thin binary: parse args, dispatch, map to an exit code.
src/cli.rs          \
src/commands/        |  binary-only modules. Compiled into the binary, NOT
src/logging.rs       |  part of the library — a CLI concern physically cannot
src/command_error.rs/   reach the engine because the engine never sees them.
```

**The rule: behaviour lives in the library (`lib.rs` and the modules it
declares). Binary-only modules may parse arguments and render results —
nothing more.** The binary reaches the engine via `use ynotes::…`; the engine
never references a binary module. One crate is the lean shape Vercel's own CLIs
use; the library boundary inside it gives separation without the ceremony, at
the cost of being discipline-enforced rather than compiler-enforced.

### Where new work goes

- **Anchoring / selector-ladder engine** → new modules in `lib.rs` (e.g.
  `anchor`, `selector`, `store`). Never in a binary-only module.
- **MCP server for agents** → when real, lift `lib.rs` + its modules into a
  `ynotes-core` crate, make this a workspace, add `ynotes-mcp` /
  `ynotes-cli` beside it. The boundary already exists, so this is a
  mechanical move. Do it only when the second front-end exists — not before.
- **New CLI subcommand** → a variant in `cli.rs::Command`, a module under
  `commands/`, one arm in `commands::dispatch`. The body calls the engine.

### Why the engine is synchronous

Filesystem reads and content matching are synchronous. The engine stays sync
(like `jj-lib`); async (`tokio`) enters only at a future network boundary (the
MCP crate). Do not pull `tokio` into the library.

## Error strategy

- The library returns a typed [`ynotes::Error`] (`thiserror`,
  `#[non_exhaustive]`). Variants distinguish what a caller must *react to*.
- The binary's `command_error` module wraps failures in `CommandError`, whose
  only job is presentational: the user-facing message and the exit code
  (`2` usage, `1` otherwise). Mirrors jujutsu's `command_error.rs`.

Typed/matchable in the library; presentational/exit-coded in the binary.

## Code style

- Never `unsafe` (workspace-forbidden).
- CLI flags are kebab-case (`--line-range`, never `--lineRange`).
- No emojis in code, output, or docs. Unicode marks (✓ ✗ → ⚠) are fine.
- Diagnostics go to **stderr** via `tracing`; stdout is data only.
- No hardcoded ANSI; honour `NO_COLOR`. (Add a colour module if/when needed.)
- British English in prose and docs.

## The comment doctrine

Reviewed as strictly as code. Standard from the official
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/):

- `//!` opens every module with *why it exists* and any invariant it holds.
- `///` on every public item; `Result` returners get `# Errors`, panicking
  functions `# Panics`, non-trivial APIs `# Examples` (compiled as doctests,
  so they cannot rot).
- `//` inline only for the non-obvious *why*. Never restate the code.
- Broken intra-doc links are a hard CI failure.

Test: if removing a comment loses nothing a competent Rust reader couldn't
recover from the code in five seconds, remove it. If it encodes a decision, a
constraint, or a citation, keep it. Assume every line is read by many people.

## Documentation propagation

Changing a flag, subcommand, or user-visible behaviour means updating **all**
of these in the same change — agents must not leave any stale:

1. `src/cli.rs` — the clap `about`/doc strings and `--help` text.
2. `README.md` — usage/examples.
3. `CHANGELOG.md` — an entry under `[Unreleased]`.
4. `AGENTS.md` — if a rule, command, or invariant changed.
5. The relevant `///` / `//!` doc comments.

## Invariants (do not break)

1. No `unsafe`.
2. No behaviour in binary-only modules; it belongs in the library.
3. The library stays synchronous.
4. **User-authored context is never silently dropped.** A failed anchor must
   surface flagged, not vanish. This is the product's core promise.
5. MSRV is 1.85, declared in `Cargo.toml`, `clippy.toml`, and `ci.yml` — keep
   the three in sync.

## Reference codebases

`.opensrc/RUST_REFERENCES.md` is the curated index of production Rust CLIs this
project learns from (ripgrep, gitoxide, jujutsu, bat, opensrc, agent-browser),
mapping each sub-problem to the specific upstream slice to study. **All of
`.opensrc/` is local-only and git-ignored** — never commit it.

`opensrc` caches source under `OPENSRC_HOME`. It must point at this project, or
it silently writes to the global `~/.opensrc` and the manifest paths go stale:

```sh
export OPENSRC_HOME="$(git rev-parse --show-toplevel)/.opensrc"
opensrc fetch <owner/repo>             # cache a repo (or crates:<name>)
rg "pattern" "$(opensrc path <owner/repo>)"   # read cached source in place
```

Releases are owned by cargo-dist (`dist init` generates `release.yml` and the
`[package.metadata.dist]` block — do not hand-edit either).
