# AGENTS.md

Instructions for any agent or contributor working in this repository. Read this
before changing code — it encodes decisions that are expensive to reverse.

## What ynotes is

A CLI that stores context for regions of code and returns, on demand,
only the context overlapping a queried line range. Not a comment system
(context costs no tokens until asked for) and not a notes vault (context is
anchored to the code and re-anchors when the code moves). The defining hard
problem — re-anchoring across edits with explicit staleness rather than silent
loss — is solved by the selector ladder (`anchor`): independent relocation
rungs whose verdict is an honest `anchored` / `drifted` / `orphaned` status,
with an unlocatable region surfaced `orphaned` rather than dropped.

## Commands

```sh
just                 # list every task
just ci              # the gate: fmt(rust+toml) · clippy -D warnings · docs · tests · deny · version-sync
cargo run -- doctor  # run the binary
cargo nextest run --all-features   # tests (doctests: cargo test --doc)
cargo clippy --all-targets --all-features -- -D warnings
cargo insta review   # accept changed snapshots after intentional output changes
```

`just ci` is exactly what CI enforces. Green locally ⇒ green in CI. Nothing is
merged or committed without it passing.

## Architecture (and the one rule)

One crate, two faces:

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
never references a binary module. One crate is the lean shape; the library
boundary inside it gives separation without the ceremony, at
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

Filesystem reads and content matching are synchronous. The engine stays sync;
async (`tokio`) enters only at a future network boundary (the
MCP crate). Do not pull `tokio` into the library.

## Error strategy

- The library returns a typed [`ynotes::Error`] (`thiserror`,
  `#[non_exhaustive]`). Variants distinguish what a caller must *react to*.
- The binary's `command_error` module wraps failures in `CommandError`, whose
  only job is presentational: the user-facing message and the exit code
  (`2` usage, `1` otherwise).

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
6. `ynotes.schema.json` — if the `save`/`query`/`list` `--json` output changed.

## Invariants (do not break)

1. No `unsafe`.
2. No behaviour in binary-only modules; it belongs in the library.
3. The library stays synchronous.
4. **User-authored context is never silently dropped.** A failed anchor must
   surface flagged, not vanish. This is the product's core promise.
5. MSRV is 1.85, declared in `Cargo.toml`, `clippy.toml`, and `ci.yml` — keep
   the three in sync.
6. **A note id is a content hash of `(target, scope, bundle, body)` — never
   a timestamp.** This makes `save` idempotent and ids deterministic, which
   agent tool-retries and tests depend on. Do not reintroduce time/randomness
   into identity.
7. **The `--json` schema is a stable, versioned agent contract.** Any change
   to a field bumps `"v"` and, in the same change, updates **both** the
   `tests/agent_contract.rs` snapshot and the published `ynotes.schema.json`;
   it is never altered silently. `tests/schema_conformance.rs` validates every
   command's real `--json` output against `ynotes.schema.json`, so a schema that
   drifts from what the binary emits fails CI — the schema cannot rot unnoticed.
8. **A resolve never writes; only `reanchor` does.** `query`/`list` are pure
   (read-only mounts, concurrent readers, a future read-only MCP server
   depend on it). `reanchor` is the sole resolve-time write path and refuses
   to refresh an `orphaned` note — re-anchoring is irreversible.
9. **The crate version in `Cargo.toml` is the single source of truth.**
   `package.json` and every `npm/*/package.json` are generated from it by
   `scripts/sync-version.mjs`; the `version-sync` CI job fails on drift. Never
   hand-edit a version in an npm manifest.
10. **A note only relocates to a renamed path when a content rung corroborates
    the region there.** git's rename signal proposes the destination; a note is
    moved (in `reanchor`) or surfaced there (in `query`) only if the ladder
    then locates the region at the new path. A rename where the region was
    deleted resolves `orphaned` and the note stays put — never welded onto the
    renamed file at an unrelated line. This is the rename-side face of #4.

## Releases

The release pipeline is **hand-written** (`.github/workflows/release.yml`) —
deliberately not cargo-dist. This keeps the workflow transparent and
`zizmor`-auditable, and avoids depending on an external release tool. Do not
reintroduce cargo-dist.

**To cut a release:**

1. Bump `version` in `Cargo.toml`; run `just sync-version` to propagate it
   into the npm manifests.
2. Rename `## [Unreleased]` in `CHANGELOG.md` to `## [X.Y.Z]` — the workflow
   extracts that section verbatim as the GitHub Release notes and **fails if
   it is absent**.
3. Commit, then push a `vX.Y.Z` tag. The tag version must equal the
   `Cargo.toml` version or the `guard` job rejects the release.

The workflow then builds all seven targets on native runners (glibc and musl
Linux x64/arm64, macOS x64/arm64, Windows x64), verifies each binary, creates
the GitHub Release (archives + `SHA256SUMS`), and publishes to npm.

ynotes is **not** published to crates.io — `Cargo.toml` sets `publish = false`,
so `cargo publish` refuses. Distribution is npm (for everyone, including
agents) and the prebuilt GitHub Release binaries; building from source with
`cargo` stays available but is not a registry channel. Do not add a
`cargo publish` step or remove `publish = false` without a deliberate decision
to reverse this.

**One-time repository setup:** a GitHub Environment named `release`, carrying
the secret `NPM_TOKEN` (the `publish-npm` job is scoped to it).

**npm distribution shape:** `ynotes` is a thin launcher (`bin/ynotes.js`)
declaring seven `optionalDependencies` — `ynotes-<platform>` — each carrying
one prebuilt binary with `os`/`cpu` (and `libc` on the Linux packages) set, so
npm installs only the matching one. The launcher
detects glibc vs musl, locates that binary, and execs it. The CI `install-test`
job exercises this end-to-end on Linux/macOS/Windows, and `install-test-musl`
does the same inside an Alpine container for both musl targets.
