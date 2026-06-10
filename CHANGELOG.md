# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Pre-1.0, minor versions may contain breaking changes; those will be called out
explicitly here.

## [Unreleased]

### Added

- Initial single-crate scaffold (`bat` model): `src/lib.rs` engine + thin
  `src/main.rs` binary, with a library boundary so the engine can later be
  extracted into its own crate without a rewrite.
- CLI surface with `doctor` and `completions` subcommands, structured exit
  codes, and `tracing`-based logging to stderr.
- Project tooling: crate lints, rustfmt/clippy/taplo/cargo-deny configuration,
  `cargo-nextest` + `insta` testing, a `justfile`, and CI (format, clippy,
  cross-platform tests, MSRV, supply-chain audit, `zizmor` Actions analysis).
- `AGENTS.md` — the canonical agent guide (architecture, error strategy,
  comment doctrine, commands, code style, documentation-propagation checklist).
  `CLAUDE.md` is a symlink to it: both filenames work, one source of truth, no
  drift.
- The note store (engine): a content-addressed `.ynotes/` directory (immutable
  one-note-per-file JSON records, rebuildable by-path index, atomic writes),
  the selector bundle captured per note (exact-quote + position floor; git and
  tree-sitter rungs land in later phases), and store discovery via
  `$YNOTES_DIR` or an upward walk for `.ynotes/HEAD`.
- `ynotes init` — create a `.ynotes` store in the current directory.
- The selector-ladder resolver (engine): R1 git transport (shells out to the
  `git` binary — no `gix`/`git2` dependency — and translates a saved range
  through the commit→work-tree diff, with rename detection), R2 exact-quote
  relocation (prefix/suffix disambiguation), and R5 position as a weak
  tie-breaker. Outcomes are clustered and combined into an explicit
  `anchored` / `drifted` / `orphaned` status with a 0–100 confidence;
  position-only evidence never anchors a note, and an unlocatable note is
  surfaced orphaned, never dropped.
- `ynotes save <file> [LINE|START:END] -m <msg>` — capture a note; the
  location form sets the scope (file / line / range); the body may come from
  stdin.
- `ynotes query <file> [LINE|START:END] [--json]` — return the context
  overlapping a file or region, in a text format or a stable versioned JSON
  schema; orphaned notes are always included.
- R3 fuzzy alignment: a region whose *contents* were edited is relocated by
  anchoring on its rarest surviving line and scoring candidates with a
  hand-rolled line-LCS similarity (no diff-crate dependency), with a
  conservative acceptance threshold so a genuinely different block honestly
  orphans rather than false-matching. A single-line region uses a bounded
  character-similarity scan. `anchored` is now reserved for a *located and
  intact* region (exact-quote or clean git transport, unmoved); an edited or
  moved region is `drifted`, even when its line numbers are unchanged.
- Agentic hardening: note ids are content-addressed over
  `(target, scope, anchor, body)` with no timestamp, so `save` is idempotent
  (a retried identical save is a no-op returning the same id with
  `"created": false`; editing the body supersedes with a new note) and ids
  are deterministic across machines. `ynotes save --json` emits a capturable
  `{id,target,scope,range,created}` record; `query --json` gains a `stale`
  convenience boolean alongside `status`/`confidence`. The `--json` schema is
  versioned and snapshot-tested as a stable agent contract.
- R4 structural anchoring (tree-sitter, always on, broad grammar set: Rust,
  JS/TS/TSX, Python, Go, Java, C/C++, Ruby, Bash, JSON, HTML, CSS — all
  MIT/Apache, `cargo deny` clean). A note stores the named-construct path
  (`module → fn parseConfig → …`) plus a structure-only subtree fingerprint
  and relocates by name in the re-parsed tree, so it survives a wholesale
  reformat or a large move that defeats the text rungs. An identical subtree
  is `anchored` (e.g. an indentation-only change); a changed body is
  `drifted`; an unknown/unparseable language contributes nothing and the
  ladder is unaffected.
- `ynotes list [file] [--json]` — every note in the store with its current
  anchor status (read-only inventory; each target read once).
- `ynotes reanchor [--dry-run]` — the deliberate, auditable pass that
  refreshes selectors for high-confidence notes so future queries anchor
  from current code, not ever-staler originals (ynotes's `git gc`). Reads
  never write; this does. Only notes that resolve at confidence ≥ 90 are
  refreshed (idempotent, supersede semantics preserving `created_at`);
  orphaned and low-confidence notes are left untouched and reported for a
  human — re-anchoring is irreversible, so it must be near-certain.
- `ynotes query --explain` — adds each rung's outcome (the agreement vector)
  to text and, as an optional `rungs` field, to JSON (default schema
  unchanged).
- Terminal colour for the human text format, suppressed when `NO_COLOR` is
  set or stdout is not a tty (JSON is never coloured).
- Bundle construction consolidated into `SelectorBundle::capture_full`, so
  `save` and `reanchor` cannot diverge and no anchoring logic lives in a
  binary-only module.
- `ynotes doctor` now reports the environment ynotes resolves against — the
  `git` binary's version (R1's optional runtime dependency; its absence is
  called out explicitly, since it disables the git-transport rung) and the
  discovered `.ynotes` store with its note count — not only the ynotes
  version and platform.
- `ynotes.schema.json` — a published JSON Schema (draft 2020-12) for the
  `save`/`query`/`list` `--json` output. The snapshot test
  (`tests/agent_contract.rs`) still locks the contract; the schema makes it
  consumable, and is now part of the documentation-propagation checklist.
- Hand-written release pipeline (`.github/workflows/release.yml`): pushing a
  `vX.Y.Z` tag builds all seven targets on native runners — glibc and musl
  Linux (x64/arm64), macOS (x64/arm64), Windows x64 — verifies each binary
  (size + smoke-run), and publishes to GitHub Releases (archives +
  `SHA256SUMS`) and npm. A `guard` job refuses any tag whose version disagrees
  with `Cargo.toml`. The crate sets `publish = false` — ynotes is deliberately
  not published to crates.io.
- npm distribution: `ynotes` is a thin launcher (`bin/ynotes.js`) that declares
  seven `optionalDependencies` — `ynotes-<platform>` — each carrying one
  prebuilt binary tagged with `os`/`cpu`/`libc`, so an install pulls only the
  binary it needs (the esbuild model). The launcher detects glibc vs musl, so
  Alpine and other musl distros are supported. `scripts/sync-version.mjs` keeps
  `package.json` and `npm/*/package.json` in lockstep with the crate version.
- CI gains a `docs` job (broken intra-doc links now fail the build, enforcing
  the comment doctrine), a `version-sync` job, and install smoke tests that
  build, pack, and `npm install` the CLI to exercise the real distribution
  path: `install-test` on Linux/macOS/Windows, and `install-test-musl` inside
  an Alpine container for both musl targets.

[Unreleased]: https://github.com/ars-2107/ynotes/commits/main
