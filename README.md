# ynotes

> Store code context that survives edits, and hand an agent only the context it asks for.

**Status:** pre-1.0, pre-release. The engine is implemented and tested — the
selector ladder, the `.ynotes` store, and every subcommand — but the API and
on-disk format may still change between minor versions until 1.0. See
[`CHANGELOG.md`](CHANGELOG.md) for what has landed and [`AGENTS.md`](AGENTS.md)
for the design.

## What this is

ynotes is a CLI for attaching context to regions of code. Unlike
comments, the context is not carried inside the file (so it costs an agent no
tokens until requested); unlike a notes vault, it is anchored to the code and
re-anchors itself when the code moves. Retrieval is range-based: ask for any
line range and ynotes returns the context overlapping it.

The hard problem — re-anchoring context across edits without it going silently
stale — is the heart of ynotes. It is solved by a ladder of independent
relocation signals (git-history transport, tree-sitter structural identity,
exact-quote and fuzzy text matching): the content signals decide whether a
region is still present and intact, and a region that cannot be located is
surfaced `orphaned` — never silently dropped.

## Layout

One crate, two faces:

```
src/lib.rs        the engine — all behaviour lives here (the library)
src/main.rs       a thin binary: parse args, dispatch, exit code
src/cli.rs …      binary-only modules (not part of the library)
```

A library boundary *inside* one crate keeps the start lean while making the
eventual extraction into a `ynotes-core`
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
cargo run --bin ynotes -- doctor --json              # machine-readable health check
cargo run --bin ynotes -- completions zsh
cargo run --bin ynotes -- init                       # create a .ynotes store
cargo run --bin ynotes -- save src/lib.rs 40:78 -m "why this matters"
cargo run --bin ynotes -- query src/lib.rs 50        # context overlapping line 50
cargo run --bin ynotes -- query src/lib.rs --json    # machine-readable
cargo run --bin ynotes -- query src/lib.rs 50 --explain  # show every rung
cargo run --bin ynotes -- list                       # all notes + status
cargo run --bin ynotes -- reanchor --dry-run         # preview a refresh
cargo run --bin ynotes -- reanchor                   # apply re-anchors
cargo run --bin ynotes -- reanchor --json            # machine-readable refresh report
cargo run --bin ynotes -- update abc1234 -m "new body" # replace a note's body in place
cargo run --bin ynotes -- delete abc1234 def5678     # remove notes by id / hex prefix
cargo run --bin ynotes -- prune --dry-run            # preview orphan cleanup
cargo run --bin ynotes -- prune                      # remove every orphaned note
cargo run --bin ynotes -- reindex --dry-run          # preview an index rebuild
cargo run --bin ynotes -- reindex                    # rebuild the by-path index from disk
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

### When to reach for it

ynotes pays off on **repeat visits** to a piece of code. Use it when:

- You're leaving breadcrumbs for an agent (or human) on a later task — a
  hidden invariant, a non-obvious failure mode, the *why* a piece of code
  looks redundant. The note costs no tokens until something queries the
  range.
- Multiple agents may work in the same area and benefit from each other's
  findings — the store is the durable communication channel.
- You're about to delete or rewrite something whose history you want
  recoverable from inside the codebase (without `git log`-spelunking).

Skip it when:

- The task is a one-shot read whose output (a PR, a summary, an explanation)
  is the durable artefact. Inline `path:line` citations in the response are
  the right answer there.
- Nobody — agent or human — will query the same code again soon.

The selection rule: *would anything change for me if I came back to this
file in a week?* If yes, leave a note. If no, skip.

### How to drive it

- **Always use `--json`.** Every JSON output rides inside the same envelope —
  `{success: true, v: 6, data: …}` on success, `{success: false, v: 6,
  error: …, type: …}` on failure — so a consumer always parses one shape
  and branches on `success`. The schema is versioned, snapshot-locked, and
  published as [`ynotes.schema.json`](ynotes.schema.json) (JSON Schema
  draft 2020-12), so it will not silently change.
- **`save` is idempotent.** A note's id is a content hash of
  `(target, scope, anchor, body)` — no timestamp. Re-running an identical
  `save` returns the same id with `"created": false` instead of duplicating,
  so retrying a tool call is safe. Editing the body makes a *new* note that
  supersedes the old — the earlier note at that location is retired, not left
  beside it.
- **Note ids rotate on `reanchor` and `update`.** The id is content-hashed
  over `(target, scope, bundle, body)`, so refreshing the anchor or replacing
  the body changes the id. To cache a stable handle to a note across
  reanchors, use the `(target, scope, body)` tuple — not the id.
- **Branch on `status`.** Each queried note carries `status` —
  `anchored`, `drifted`, or `orphaned` — and a convenience `stale` boolean
  (`false` only for `anchored`). There is no numeric confidence: the category
  is the signal. Suggested policy:

  | observed | meaning | suggested agent action |
  |---|---|---|
  | `status: "anchored"` | located and intact | trust the context as-is |
  | `status: "drifted"` | found, but code moved/changed | use it, but re-read the lines; consider `ynotes update <id>` |
  | `status: "orphaned"` | code gone or rewritten | treat the note as historical; surface to the human / re-`save` |

- **Exit codes:** `0` = success *even if there are no notes or only orphans*
  (inspect `data.notes[]` — empty is not an error; an orphan-only result is
  also exit 0); `2` = bad invocation (including a path outside the store);
  `1` = real failure (and `delete`'s partial-failure case — see below).
- **`--json` always writes the envelope to stdout** — including on failure.
  Branch on the `success` field, not the exit code: a `2` (usage) or `1`
  (engine/io/render) failure carries a `{success: false, error, type}`
  envelope (where `type` is one of `usage`, `engine`, `io`, `render`), so the
  response is parseable in both paths. On a `0` exit, `data.warnings[]`
  carries non-fatal advisories (e.g. a missing target file: `notes` empty,
  the path issue in-band).
  - **`delete` is the one exception to "branch on `success`".** When some
    requested ids are `ambiguous` or `not_found`, `delete` emits a *success*
    envelope (`success: true`) yet exits `1`: the envelope describes the
    partition, the exit code reports the partial failure. To tell a
    fully-clean delete from a partial one, check that `data.ambiguous` and
    `data.not_found` are both empty — neither `success` nor the exit code
    alone is sufficient.
- **`query` returns one `notes` array.** Since `v=5`, `query --json` carries
  `{query, notes, warnings}` — every resolved note in a single array, with
  `status` discriminating. Group by `status == "orphaned"` if you want the
  historical matched-vs-orphaned split; orphans are still always returned (the
  never-drop promise).
- **`query` never writes; `reanchor` is the only write-on-resolve path.** An
  agent can `query` freely (idempotent, safe to parallelise). When notes have
  drifted, run `reanchor` (optionally `--dry-run` first) as a deliberate
  maintenance step — it refreshes only high-confidence notes and leaves
  orphans for a human. `query --explain` exposes the per-rung agreement
  vector for debugging an anchor.
- **`update`, `delete`, `prune` round out the lifecycle.** `update <id>`
  replaces a note's body in place (re-anchoring against current code).
  `delete <id>...` removes notes by full id or unambiguous hex prefix
  (≥4 chars); a prefix that matches multiple notes is refused, never
  silently fans out. `prune` removes every orphan in one pass (use
  `--dry-run` first to preview).
- **`reindex` repairs the index, not the notes.** The notes under
  `.ynotes/notes/` are the source of truth; `.ynotes/index/by-path.json` is a
  cache that maps each file to its note ids. If that cache is lost,
  hand-edited, or out of step — a `query` or `delete` that logs "run a
  reindex" is the tell — `reindex` rebuilds it from disk: it recovers any note
  the index forgot and drops any pointer with no note behind it, reporting both
  in `--json` (`recovered`/`dangling`/`malformed`). It never reads the anchor
  ladder and never touches a note, so it cannot lose context or rotate an id.

### `ynotes lookup [--target <path>] [--body-contains <text>] [--json]`

Find note(s) by a stable handle and print their current id(s). A note's id
rotates on `reanchor`/`update`, so to reference a note durably (PR, comment),
record the `(target, body)` and resolve it back to the live id:

    ynotes lookup --target src/auth.rs --body-contains "token refresh"

At least one of `--target` / `--body-contains` is required. `--body-contains`
is a case-sensitive substring. Read-only; no match still exits 0.

### Understanding `--explain` scores

`query --explain` (and the `rungs` field under `--json --explain`) surfaces
each selector-ladder rung's outcome, including a self-assessed score in
`[0, 100]`. **There is no combined "confidence" number** — the verdict is
the categorical `status`. The per-rung scores are diagnostic: useful when
debugging why a note resolved the way it did. Scale per rung:

| rung | score | what it means |
|---|---|---|
| `git` | 100 | the range is unchanged in the working tree |
| `git` | 90 | git transported the range to a new location, and the touched lines did not overlap it |
| `git` | 70 | transported, but the moved range was itself touched (refactor inside it) |
| `quote` | 95 | the exact text occurs once in the file |
| `quote` | 90 | the exact text occurs multiple times; the surrounding `prefix`/`suffix` lines disambiguated which one |
| `quote` | 55 | multiple matches, none disambiguated by context — fell back to the occurrence nearest the saved line |
| `structural` | 95 | tree-sitter found the enclosing construct *and* its content fingerprint (node kinds + identifier/literal text) is identical — intact bar whitespace and comments |
| `structural` | 55–94 | construct found, fingerprint drifted (linear in similarity 0.5–1.0) |
| `fuzzy` | 45–80 | line-LCS alignment matched, scaled by similarity above the acceptance threshold. **Capped below 90** — a fuzzy match alone is never `anchored` |
| `position` | 20 | the saved range still fits the file. **Diagnostic only** — never consulted by the verdict, since it is the saved range itself |
| `position` | 10 | the file shrank; the saved range was clamped to fit |

A `result: "miss"` means the rung ran and found nothing. `result: "skipped"`
means the rung could not run — typically `git` when the file is not in a
repo, or `structural` for an unsupported language — and contributes nothing
to the verdict, but is not held against it.

With [`just`](https://github.com/casey/just): `just` lists every task;
`just ci` runs the exact gate CI enforces (format, lint, docs, test, audit,
version-sync). Tests use [`cargo-nextest`](https://nexte.st). Releases are cut
by a hand-written workflow — push a `vX.Y.Z` tag; see [`AGENTS.md`](AGENTS.md).

## Sharing notes across a team

`.ynotes/` is built to be committed and merged like docs. Commit it alongside
your code and push.

The store auto-writes a `.ynotes/.gitattributes` with two rules, requiring zero
per-clone setup:

- `index/by-path.json merge=union` — the index is a union-mergeable JSONL
  cache, so concurrent note-adds on parallel branches merge into a valid index
  with no conflicts.
- `notes/** -merge` — note files are never textually merged by git. A
  concurrent edit cannot inject conflict markers and corrupt the JSON.

**Two devs adding different notes** merge cleanly; nothing is lost.

**Two devs reanchoring/updating the same note, or saving the same note**
produces a git conflict on the note files (rename/rename or add/add, because
git sees two versions of the same path). The files stay valid JSON. Resolve by
keeping both (`git add .ynotes/`), commit, then run `ynotes reanchor`. Because
the id is a content hash, two copies that re-anchor to the same place get the
same id and collapse into one note automatically.

**Recovery.** If the index is ever malformed or missing, `ynotes reindex`
rebuilds it from `notes/` (the source of truth) in one command. It no longer
refuses on a mangled index, and a missing index is reported rather than
silently read as "no notes". One caveat: do not run `ynotes reindex`
mid-merge (before resolving), as writing the index while git is mid-merge
complicates `git merge --abort`; resolve the merge first.

Workflow: commit `.ynotes/`, push, and merge like any other change.

## License

Dual-licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT)
at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
