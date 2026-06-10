# ynotes

> Store code context that survives edits, and hand an agent only the context it asks for.

**Status:** pre-1.0, pre-release. The engine is implemented and tested — the
selector ladder, the `.ynotes` store, and every subcommand — but the API and
on-disk format may still change between minor versions until 1.0. See
[`CHANGELOG.md`](CHANGELOG.md) for what has landed and [`AGENTS.md`](AGENTS.md)
for the design.

## What this is

ynotes is a git-like CLI for attaching context to regions of code. Unlike
comments, the context is not carried inside the file (so it costs an agent no
tokens until requested); unlike a notes vault, it is anchored to the code and
re-anchors itself when the code moves. Retrieval is range-based: ask for any
line range and ynotes returns the context overlapping it.

The hard problem — re-anchoring context across edits without it going silently
stale — is the heart of ynotes. It is solved by a ladder of independent
relocation signals (git-history transport, tree-sitter structural identity,
exact-quote and fuzzy text matching) whose *agreement* is the confidence
score. A region that cannot be relocated is surfaced `orphaned` — never
silently dropped.

## Layout

One crate, two faces (the `bat` model):

```
src/lib.rs        the engine — all behaviour lives here (the library)
src/main.rs       a thin binary: parse args, dispatch, exit code
src/cli.rs …      binary-only modules (not part of the library)
```

A library boundary *inside* one crate keeps the start lean (the shape Vercel's
own Rust CLIs use) while making the eventual extraction into a `ynotes-core`
crate — the day an MCP server needs the engine too — a mechanical move rather
than a rewrite. Full rationale in [`AGENTS.md`](AGENTS.md).

## Install

```sh
# npm — works with pnpm, yarn, and bun too (they all resolve from the npm
# registry). Pulls only the prebuilt binary for your platform.
npm install -g ynotes
npx ynotes doctor
```

Prebuilt binaries and checksums for every release are also attached to the
[GitHub Releases](https://github.com/ars-2107/ynotes/releases) page. ynotes is
not published to crates.io; to build from a Rust toolchain, see below.

> Pre-release: the package names above are reserved by the release pipeline
> but nothing is published until the first `vX.Y.Z` tag. Until then, build
> from source as below.

## Build and run

Requires a Rust toolchain (see `rust-toolchain.toml`; MSRV 1.85).

```sh
cargo run --bin ynotes -- doctor
cargo run --bin ynotes -- completions zsh
cargo run --bin ynotes -- init                       # create a .ynotes store
cargo run --bin ynotes -- save src/lib.rs 40:78 -m "why this matters"
cargo run --bin ynotes -- query src/lib.rs 50        # context overlapping line 50
cargo run --bin ynotes -- query src/lib.rs --json    # machine-readable
cargo run --bin ynotes -- query src/lib.rs 50 --explain  # show every rung
cargo run --bin ynotes -- list                       # all notes + status
cargo run --bin ynotes -- reanchor --dry-run         # preview a refresh
cargo run --bin ynotes -- reanchor                   # persist confident re-anchors
```

A saved note is not pinned to line 40:78. When the file is edited, `query`
re-anchors it through a ladder of independent signals — git history
transport, tree-sitter structural identity (survives reformatting and large
moves), exact-text and fuzzy relocation — and reports its status:
`anchored`, `drifted` (found, but moved/changed), or `orphaned` (could not be
located; surfaced anyway with its last known position, never silently
dropped).

## For coding agents (Claude Code, Codex, OpenCode, …)

ynotes is built to be driven by an LLM coding agent, not just a human.

- **Always use `--json`.** `save`, `query`, and `list` accept it; the schema
  is versioned (`"v"` on the `query`/`list` shapes), snapshot-locked, and
  published as [`ynotes.schema.json`](ynotes.schema.json) (JSON Schema draft
  2020-12), so it will not silently change.
- **`save` is idempotent.** A note's id is a content hash of
  `(target, scope, anchor, body)` — no timestamp. Re-running an identical
  `save` returns the same id with `"created": false` instead of duplicating,
  so retrying a tool call is safe. Editing the body makes a *new* note
  (supersede semantics).
- **Branch on `stale`.** Each queried note carries `status`
  (`anchored`/`drifted`/`orphaned`), `confidence` (0–100), and a convenience
  `stale` boolean. Suggested policy:

  | observed | meaning | suggested agent action |
  |---|---|---|
  | `stale: false` | anchored & confident | trust the context as-is |
  | `status: "drifted"` | found, but code moved/changed | use it, but re-read the lines; consider updating the note |
  | `status: "orphaned"` | anchor lost | surface to the human / re-`save` against the new code |

- **Exit codes:** `0` = success *even if there are no notes or only orphans*
  (inspect the `matched`/`orphaned` arrays — empty is not an error); `2` =
  bad invocation; `1` = real failure.
- **`query` never writes; `reanchor` is the only write-on-resolve path.** An
  agent can `query` freely (idempotent, safe to parallelise). When notes have
  drifted, run `reanchor` (optionally `--dry-run` first) as a deliberate
  maintenance step — it refreshes only high-confidence notes and leaves
  orphans for a human. `query --explain` exposes the per-rung agreement
  vector for debugging an anchor.

With [`just`](https://github.com/casey/just): `just` lists every task;
`just ci` runs the exact gate CI enforces (format, lint, docs, test, audit,
version-sync). Tests use [`cargo-nextest`](https://nexte.st). Releases are cut
by a hand-written workflow — push a `vX.Y.Z` tag; see [`AGENTS.md`](AGENTS.md).

## License

Dual-licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT)
at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
