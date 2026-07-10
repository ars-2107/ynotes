# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Pre-1.0, minor versions may contain breaking changes; those will be called out
explicitly here.

## [Unreleased]

### Added
- **Quote-verified saves**: `ynotes save` gains `--code <TEXT>` and the MCP
  `remember` tool gains `code` — quote the region's text alongside the line
  numbers and the save verifies the location against the file before
  anchoring. Coordinates the quote corroborates save as given; stale
  coordinates are corrected when the quote matches exactly once elsewhere
  (the payload's `range` reports where the note landed, and the human
  confirmation line marks the move with `(corrected from line N)`); a quote
  found nowhere, or at several places none of which is the declared start,
  refuses with a typed usage error (`CodeNotFound` / `CodeAmbiguous`, exit
  `2`) whose message carries the one-step recovery — the ambiguous message
  lists at most ten candidate lines (plus an "and N more" count), and a
  corrected span that would end past EOF names the line the quote matched,
  so the fix is to re-declare from that line. Matching trims each line
  (indentation and trailing whitespace are ignored, the text is not) and is
  otherwise exact — fuzzy matching stays a resolve-time-only mechanism. The
  region's length comes from `START:END` / `end` when given, else from the
  quote's line count, so a long region needs only its opening lines quoted.
  The `--json` / tool payload shape is unchanged (`saveData`, contract v12);
  the MCP input schemas now advertise `code` and `minimum: 1` on
  `start`/`end`, and the server instructions teach quote-anchored saves as
  protocol step 3.
- **`ynotes mcp`**: run ynotes as a stdio [MCP](https://modelcontextprotocol.io)
  server for agent clients (Claude Code, Codex, Cursor, …). Register it once
  (e.g. `claude mcp add ynotes -- ynotes mcp`) and the client spawns the process
  per session; it completes the MCP initialize handshake and injects the loop
  protocol as the server `instructions`. It exposes five tools, each a stdio
  mirror of a CLI command returning the same versioned `--json` payload:
  **`recall`** returns the context notes overlapping a file or line region — a
  read-only mirror of `query`, with `count: true` for a cheap existence check
  and a `{"store":"absent"}` success in a repo with no store; **`remember`**
  saves (or supersedes) a note anchored to a region — a mirror of `save`,
  content-addressed so a retried call is idempotent, and the only tool that
  bootstraps the store, creating one at the git root on first use; **`forget`**
  deletes a note by id (or unambiguous hex prefix) — a mirror of `delete` with a
  `dry_run` preview, returning the same partitioned `deleteData`; a request that
  resolves to nothing or to more than one note is surfaced as an error result
  carrying that partition, never silently dropped, and a corrupt record is
  removable only by its exact 64-character id; **`notes`** browses the store —
  one note by id, a case-sensitive body-substring search, or the whole inventory
  with each note's anchor status, plus `count: true` for a store-wide status
  breakdown, and a `{"store":"absent"}` success in a repo with no store;
  **`reanchor`** persists re-anchors for every note that confidently moved — a
  mirror of the `reanchor` command and the sole resolve-time write path
  (invariant #8): it refreshes selectors, follows committed file renames, and
  never touches an orphaned note, with a `dry_run` preview and the same
  `{"store":"absent"}` success in a repo with no store. The five-tool surface
  (`forget` `notes` `reanchor` `recall` `remember`) is frozen by a reviewed
  `tools/list` snapshot so any future change to a tool's name, description, or
  annotations is a reviewed diff rather than a silent drift, and validated
  end-to-end: a conformance sweep checks every tool's real over-the-wire payload
  against the published `ynotes.schema.json`, extending invariant #7's
  cannot-fork-silently guarantee to the MCP face.
  The server lives in a new `ynotes-mcp` workspace crate, the sole home of async
  (`tokio`/`rmcp`); the engine stays synchronous.
- **First `save` auto-creates the store at the git root.** Running `save` in a
  git repository with no `.ynotes` yet bootstraps one at the repository root
  (the enclosing `.git` directory *or* file, so worktrees are handled) and says
  so on the human confirmation line — adoption no longer costs a separate
  `ynotes init`. Placement is deterministic (repo root only); with no git root
  the save still fails and directs you to `ynotes init`. `$YNOTES_DIR` keeps its
  override precedence and is never a creation site — pointing it at a non-store
  is refused rather than bootstrapping there. `save --json` reports it as
  `saveData.store_created` (contract v12, below).
- **`ynotes show <id>`**: view a single note by full id or unambiguous hex
  prefix (>= 4 chars), resolved against current code — the by-id read that sat
  between `query` (by file/line) and `list` (the whole store). Shows the body
  and current anchor status; `--explain` adds the per-rung agreement vector,
  `--json` emits the `showData` payload (`{note, warnings}`). Resolves against
  the note's stored `target`, so a note whose file was renamed away reads
  `orphaned` here — `query` on the new path is the rename-aware read.
- **`--count` summary mode on `query` and `list`**: emit a status breakdown
  (`{total, anchored, drifted, orphaned}`) instead of the note bodies, so an
  agent can cheaply check whether there is context here before pulling it. The
  full, rename-aware resolution still runs; only the output is condensed.
  `--json` emits `queryCountData` / `listCountData` (with a `malformed` *count*
  in place of the array). Conflicts with `--explain`.
- **`--explain` on `list`** (previously `query`-only): populates the optional
  `rungs` field per note. No contract shape change — `rungs` was already an
  optional field on the shared note object.
- **Notes now follow file renames.** `reanchor` migrates a note whose target
  file was renamed **and committed** to the new path, after a content rung
  confirms the region is there — reported under a new always-present
  `relocated[]` array in `reanchor --json` (`{from_target, to_target, old_id,
  new_id, from, to}`). A rename where the region was *deleted* is not moved: it
  stays `orphaned` (the wrong-file guard — a note is never welded onto the
  renamed file at an unrelated line). A rename that is only *staged* is
  deferred, not migrated: the destination has no committed git baseline yet, so
  relocating there would strip the note's transport rung and it could never heal
  again; `query` already surfaces it at the new path meanwhile (below), and
  `reanchor` migrates it once the rename is committed.
- `query` self-heals across a rename before any maintenance runs: querying the
  new path surfaces notes from the path it was renamed from (resolved against the
  current file), flagged with `relocated_from` (`--json`) — so an agent working
  at the new path finds the context immediately. This runs even when the new
  path already has notes of its own, so a fresh note on the destination never
  hides the pre-rename context (results are de-duplicated by id). Read-only; the
  migration is made durable by `reanchor`.
- `ynotes delete <full-id>` can now **purge a corrupt record** — one that
  cannot be read, so `find_by_id_prefix` (and therefore a normal `delete`/
  `update`) could not previously reach it. Removal requires the *exact*
  64-character id (a prefix never purges one, since there is no body to preview);
  the id is the handle shown by `query`/`list`/`doctor`. Reported under a new
  always-present `deleted_unreadable[]` array in `delete --json`; an entry there
  is a success and does **not** flip the exit code.
- `ynotes lookup [--target <path>] [--body-contains <text>]`: resolve a stable
  `(target, body)` handle to a note's current id(s). `--json` payload `lookupData`.
- `doctor` now reports **store health**: malformed note records, index drift
  (`recovered`/`dangling`), and missing managed `.gitattributes` merge guards.
  `--json` carries a `health` object; the scan is read-only and lock-free.
- `query --json` and `list --json` now carry an always-present `malformed[]`
  array — note records the index points at that cannot be read or parsed
  (`{id, target, error}`), surfaced rather than dropped.

### Fixed
- **A file rename no longer orphans every note on the file and hides them at
  the new path** (the R1 rename gap). A note's `target` is its index key and
  part of its content-hash id, and nothing rewrote it, so a routine `git mv`
  made the note invisible at the new path and `orphaned` at the old one. It is
  now followed both ways: `query` surfaces it at the new path immediately (for a
  staged rename too), and `reanchor` migrates it durably once the rename is
  committed (both above). A plain unstaged `mv` to an untracked path is still out
  of scope (git emits no rename signal); stage the move so `query` follows it,
  and commit it so `reanchor` migrates it.
- **A deleted region can no longer silently drift onto a same-shaped sibling.**
  When a region's code is gone (deleted, or its old path reused for unrelated
  content after a rename), the resolver could weld the note onto a coincidental
  look-alike elsewhere in the file — which `reanchor` would then make permanent.
  The cause was that a *moved* fuzzy match's corroboration accepted a witness on
  range overlap alone, ignoring the witness's own confidence:
  - git could only transport the saved range onto its deletion point — a
    low-confidence `touched` hit git itself flags — yet that counted as a full
    witness. A git witness now requires an *untouched* transport (one that
    followed surviving lines).
  - a `structural` hit on a **same-named construct in a different scope**
    (`Bar::build` when the note was on `Foo::build`) counted at any score. A
    structural witness now requires an *exact ancestor-chain* match (the same
    construct in its original scope), not a namesake elsewhere.

  With neither a qualifying git nor structural witness, the note orphans
  honestly (invariants #4/#10) instead of drifting onto unrelated code. Genuine
  moves still drift exactly as before: a region whose boundary lines survive has
  an untouched git transport, and a reformatted-or-moved construct in its own
  scope keeps an exact-chain structural hit.
- **A note no longer relocates onto a same-line replacement across a rename.**
  Rename-following corroborates the region at the new path, but a fuzzy hit at
  the note's *original* line numbers is not evidence in a *different* file — a
  renamed file that replaces the region in place with similar-but-unrelated code
  would otherwise be accepted via the "did not move" shortcut. Relocation now
  resolves in a dedicated mode that drops that shortcut: a fuzzy-only match at
  the saved lines needs an independent witness (an untouched git transport or an
  exact-chain structural hit) or the note stays `orphaned` at its old path,
  never welded onto the replacement (invariants #4/#10). Same-file resolution is
  unchanged — an in-place edit still drifts at its own anchor.
- **Renames of files whose path contains a quote, backslash, or tab are now
  followed.** Rename detection parsed git's `--name-status` output on tab
  boundaries, but git C-quotes paths with unusual characters, so those renames
  were silently missed and their notes orphaned. Detection now uses `-z`
  (NUL-delimited, unquoted) output, so any valid path is handled.
- **A corrupt index id can no longer panic a read.** An index entry whose id is
  not a 64-character content hash (a hand-edited or damaged `by-path.json`) could
  reach the two-character fan-out split and panic. The index loader now rejects a
  non-id up front as `IndexMalformed`, surfacing the "run `ynotes reindex`" repair
  hint instead (invariant #4).
- A single malformed note file no longer hard-fails `query`/`list` (and, through
  them, `lookup`/`reanchor`/`doctor`). The bad record is skipped from the
  results and surfaced (`malformed[]` / `doctor` `health`), so one corrupt
  record can neither sink an otherwise healthy read nor hide behind it
  (invariant #4). Mirrors `reindex`'s long-standing posture.
- Note files are no longer corrupted by a team merge: the store ships
  `notes/** -merge` so git never textually merges content-addressed note JSON
  (a concurrent reanchor/update or identical save previously injected conflict
  markers and broke every read of the store).
- `ynotes reindex` now recovers a malformed index instead of failing with the
  same "run reindex" error it recommends.
- A missing index over surviving notes is now reported (`run reindex`) instead
  of silently reading as "no notes".
- The remedy a corrupt record points at is now correct. `query`/`list`/`doctor`
  previously told the user to "run `ynotes reindex`", but `reindex` rebuilds the
  index and never removes a malformed file, so the record kept resurfacing. They
  now point at `ynotes delete <full-id>`, the verb that actually clears it.
- `reanchor` no longer churns the on-disk filename of a note that did not move.
  Because the id is content-hashed over the whole selector bundle, refreshing
  volatile bundle state (the file's line count, the HEAD commit) used to rotate
  the id and rewrite the record under a new name with no logical change.
  `reanchor` now refreshes only a note that actually *relocated* (its anchor
  resolves to a different range); an unmoved note keeps its id and its file. A
  trade: an unmoved note's git baseline is no longer refreshed, so R1 transports
  it from an older commit (still correct, marginally more work).

### Changed
- `--json` agent contract bumped to `v: 12` (adds `saveData.store_created` — a
  boolean, `true` only on the `save` that bootstrapped the `.ynotes` store, so
  an agent learns the first-save auto-init happened on the same call — and adds
  the `storeAbsent` `$defs` shape a future MCP read tool will emit in a
  repository with no store; no existing field changed; mirrored in
  `tests/agent_contract.rs` and `ynotes.schema.json`).
- **Internal: the crate is now a three-crate workspace** — `crates/ynotes-core`
  (the engine, its lib target still named `ynotes`, so `use ynotes::…` is
  unchanged), `crates/ynotes-cli` (the binary), and `crates/ynotes-mcp` (the
  MCP front-end). The old library boundary inside a single crate is now a crate
  boundary the compiler enforces. No user-facing change beyond the MCP server
  and first-save auto-init above; the binary is still a single `ynotes`.
- `--json` agent contract bumped to `v: 10` (adds the always-present
  `relocated[]` array to `reanchor` and the optional `relocated_from` field to
  the shared resolved-`note` object; no existing field changed; mirrored in
  `tests/agent_contract.rs` and `ynotes.schema.json`).
- `--json` agent contract bumped to `v: 9` (adds the always-present
  `deleted_unreadable[]` array to `delete`; no existing field changed; mirrored
  in `tests/agent_contract.rs` and `ynotes.schema.json`).
- `--json` agent contract bumped to `v: 8` (adds the `malformed[]` array to
  `query`/`list` and the `health` object to `doctor`; no existing field
  changed; mirrored in `tests/agent_contract.rs` and `ynotes.schema.json`).
- `--json` agent contract bumped to `v: 7` (adds the `lookup` payload; mirrored
  in `tests/agent_contract.rs` and `ynotes.schema.json`).
- The store's `.gitattributes`/`.gitignore` are now maintained line-by-line
  (managed lines are ensured without clobbering user customisation), upgrading
  older stores to the `notes/** -merge` guard on the next `init`/`reindex`.

### Documentation

- Clarified the `--json` agent contract (schema + README) around the
  exit-code / envelope relationship: `delete` emits a `success: true`
  envelope while exiting `1` when some requested ids are `ambiguous` or
  `not_found`, so the success/failure envelope is **not** a function of the
  exit code. A fully-clean delete is identified by empty `ambiguous` and
  `not_found` arrays; `delete` is the only command with this asymmetry.
- Documented `doctorData.store.unreadable.path`: present when a `.ynotes`
  store was located but unreadable, absent when discovery itself failed (the
  implicated path is then carried in `reason`). Added discrete-`path`
  descriptions to the `found`/`unreadable` doctor variants for parity.
- Schema descriptions only — this clarification changed no field, type, or
  output and on its own needed no `v` bump. (The release's bump to `v6` comes
  from the new `reindex` payload under **Added** below.)

### Added

- Team sharing: `.ynotes/` is now safe to commit and merge. The by-path index
  is stored as sorted, union-mergeable JSONL, and a new store writes
  `.ynotes/.gitattributes` (`merge=union`) and `.ynotes/.gitignore` (the lock
  file) so concurrent note-adds on parallel branches merge cleanly with zero
  setup. A malformed index now errors with a `run ynotes reindex` hint. The
  index format is read backward-compatibly (old single-object stores still
  open). No `--json` contract change.
- `ynotes reindex` — rebuild the by-path index from the notes on disk. The
  index (`.ynotes/index/by-path.json`) is a cache; the note records under
  `.ynotes/notes/` are the source of truth. Run it when the cache is lost,
  hand-edited, or out of step with the notes — the situation the existing
  "run a reindex" log line points at. It recovers any note the index forgot
  and drops any pointer with no note behind it, never touching the notes
  themselves (so it cannot lose context or rotate an id), and skips an
  unreadable record rather than failing the rebuild. `--dry-run` previews;
  `--json` reports `scanned`/`indexed`/`recovered`/`dangling`/`malformed`.
  **Bumps the `--json` contract to `v6`** (adds the `reindexData` payload); no
  existing payload changed.
- `ynotes reanchor --json` — machine-readable refresh report. Mirrors the
  text report (`changed[]`, `skipped[]`, `unchanged`) inside the agent
  envelope; each skipped entry carries a stable `reason_code` enum
  (`orphaned` / `missing-target` / `symlink-escape`) so a consumer can
  branch without parsing prose. Lets a script run a dry-run, review, and
  apply loop without regex on the text form.

### Fixed

- Argument errors rejected by clap (an unknown subcommand, a missing required
  argument, an unexpected flag or value) now honour the `--json` agent
  contract. Previously `Cli::parse()` let clap exit the process before dispatch,
  so `ynotes save --json` (missing `<FILE>`), `ynotes bogus --json`, and the
  like exited `2` with **empty stdout** and a diagnostic on stderr —
  contradicting the schema's promise that "the envelope is emitted on stdout
  regardless of exit code". The binary now parses with `try_parse` and, when
  `--json` was requested, renders the failure envelope
  (`{"success": false, "v": 7, "type": "usage", …}`) to stdout and exits `2`.
  `--help`/`--version` (successful outputs) and the non-`--json` human path are
  unchanged. No schema `v` bump from this fix — the failure-envelope shape is
  unchanged.
- `list` and `query` text now render the note's scope alongside its range
  (`x.rs file:1:3` vs `x.rs range:1:3`), matching the shape `save` and
  `delete` already use. Previously a file-scoped note over a 3-line file and
  a range-scoped note over its full 1:3 span rendered identically — only
  `--json`'s `scope` field disambiguated, leaving the human form ambiguous.
- Structural rung (R4) `capture` no longer silently drops the rung when the
  user's saved range includes a trailing blank row after the function it
  encloses. The blank row is a sibling of the function in the AST, so the
  smallest *named* node covering the original region was the unnamed root
  and `smallest_named_containing` returned `None`. A natural human selection
  ("the whole block, including the gap before the next one") therefore
  stripped structural permanently from the note's bundle. `capture` now
  trims whitespace-only rows off both ends of the region for the AST search
  only; `rel_start`/`rel_end` stay derived from the user's literal selection
  so the trailing blank is reproduced when the construct moves.
- Structural rung (R4) now treats `impl T` and `impl Trait for T` as named
  path anchors. tree-sitter-rust binds the impl's identity to its `type` and
  `trait` fields rather than `name`, so the previous `node_name` check
  dropped the impl ancestor from every method's path. Two same-named methods
  on different types — `impl A { fn foo() {} }` and `impl B { fn foo() {} }`
  — shared the path `[function_item: foo]`, so the rung could vote for the
  wrong target. The cluster-and-dissent resolver caught this as an `orphan`
  rather than mis-anchoring, but R4 was contributing noise instead of
  signal for every Rust impl method. The fallback synthesises a
  `Trait for Type` (or bare `Type`) name from the existing fields, so
  `PathStep` stays flat and the `--json` schema is unchanged.
- `Store::relativize` (and therefore the id of every saved note) now
  canonicalises both sides before comparison. Previously, two spellings
  of the same physical file produced different note ids — `sub/file.txt`
  and `sub/../sub/file.txt` hashed to distinct ids and surfaced as
  duplicate notes; on macOS, an absolute `/tmp/...` input was rejected as
  outside a `/private/tmp/...` work tree because `/tmp` is a symlink to
  `/private/tmp`. `..` segments and symlinked roots now collapse to the
  same target string, so the content-addressed id (invariant #6) holds
  across spellings. Paths whose deepest ancestor does not exist (a
  `query`/`delete`/`list` against a removed file) still relativize via a
  lexical fallback.
- `Error::Io` display now special-cases `NotFound` (`` `<path>` does not
  exist (check the path) ``) and includes the underlying OS error for
  other I/O kinds. Previously the message was a context-free `i/o error
  at <path>`; the doc comment had always promised the OS error would
  surface alongside the path but the format string never emitted it. The
  query path's missing-file message and the save path's missing-file
  message now read the same way.
- `ynotes query` text-mode advisories now prefix with `warning:` (e.g.
  `ynotes: warning: <path> does not exist (and has no notes) — check the
  path`) so the exit-0 advisory is visually distinct from the exit-1
  `ynotes: <error>` line `main` prints. The JSON path already separated
  these via `warnings[]`.
- The symlink-escape guard on `save`/`update` no longer blames a symlink
  when none is involved. Previously any path whose canonical form sat
  outside the work tree — an absolute outside path like `/etc/hosts`, a
  character device like `/dev/null` — was rejected with
  "(symlink target escapes the work tree)" even though the trigger had
  no symlink. The guard now restricts itself to paths whose own
  `symlink_metadata` reports a symlink; non-symlink outside paths fall
  through to `Store::relativize`'s existing `OutsideStore` ("is outside
  the ynotes store at `<root>`"). The honest symlink case keeps the
  symlink-specific wording and also names the resolved target
  (`` `<path>` is a symlink whose target escapes the work tree at
  `<root>` (resolves to `<resolved>`) ``). Exit code (`2`, usage) and
  the safety guarantee itself are unchanged.

### Added

- `ynotes delete <id>...` — remove one or more notes from the store, by full
  id or any unambiguous hex prefix of at least 4 characters. Multi-id is
  atomic per-id: successful deletions
  still happen even when other ids in the same call are ambiguous or not
  found, and the process exits `1` only if anything failed. `--dry-run`
  reports what would be removed without writing. `--json` returns a
  partitioned report (`deleted` / `ambiguous` / `not_found`).
- `ynotes update <id> [-m <body>] [--json]` — replace an existing note's
  body in place. The note's target file and scope are reused; the line
  range is *re-resolved* through the anchor ladder so an `update` on a
  drifted note anchors against where the code is now. The new note
  supersedes the old (its content-hash id is retired) and `created_at` is
  preserved. Identification accepts the same hex-prefix selector as
  `delete`. The body may come from `-m` or stdin (same trim semantics as
  `save`). An identical-body update is a no-op (`id == previous_id`,
  `created: false`).
- `ynotes prune [--dry-run] [--json]` — remove every orphaned note in one
  pass. Drifted notes are never touched (`reanchor` is for them); the cleanup
  is deliberate — ynotes never auto-prunes — because an orphan may be
  historically valuable or its target temporarily missing.
- `Store::find_by_id_prefix` — engine API exposed for the new id-based
  selectors; returns every match, the count-handling lives in the binary
  (matches/none/many become the right diagnostic per command).
- `Note::with_bundle_and_body` — builder that supersedes a note with a new
  body *and* a refreshed bundle in one step, computing the content-addressed
  id exactly once over the final identity tuple. Used by `update`.
- Initial single-crate scaffold: `src/lib.rs` engine + thin
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
  tie-breaker. The content rungs (exact quote, fuzzy) decide an explicit
  `anchored` / `drifted` / `orphaned` status; git and position corroborate
  but never anchor a note alone, and an unlocatable note is surfaced
  orphaned, never dropped.
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
  character-similarity scan. `anchored` is reserved for a *located and
  intact* region — exact text present and unmoved, or an identical structural
  fingerprint; an edited or moved region is `drifted`, even when its line
  numbers are unchanged.
- Agentic hardening: note ids are content-addressed over
  `(target, scope, anchor, body)` with no timestamp, so `save` is idempotent
  (a retried identical save is a no-op returning the same id with
  `"created": false`; editing the body supersedes with a new note) and ids
  are deterministic across machines. `ynotes save --json` emits a capturable
  `{id,target,scope,range,created}` record; `query --json` gains a `stale`
  convenience boolean alongside `status`. The `--json` schema is
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
  from current code, not ever-staler originals. Reads
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
- `ynotes doctor --json` emits that same environment report — ynotes version,
  platform, `git` availability, and the discovered store path with its note
  count — as a single machine-readable JSON object, for agents and scripts
  that want a health check they can parse. The human text output is unchanged.
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
  binary it needs. The launcher detects glibc vs musl, so
  Alpine and other musl distros are supported. `scripts/sync-version.mjs` keeps
  `package.json` and `npm/*/package.json` in lockstep with the crate version.
- CI gains a `docs` job (broken intra-doc links now fail the build, enforcing
  the comment doctrine), a `version-sync` job, and install smoke tests that
  build, pack, and `npm install` the CLI to exercise the real distribution
  path: `install-test` on Linux/macOS/Windows, and `install-test-musl` inside
  an Alpine container for both musl targets.

### Changed

- **Breaking (agent contract):** the `--json` schema is bumped to `"v": 5`.
  `query --json` now returns one `notes: []` array instead of the v=4
  `{matched: [], orphaned: []}` split — every resolved note rides in one
  array with `status` ("anchored" / "drifted" / "orphaned") discriminating.
  Orphans are still always returned (the never-drop promise, invariant
  #4); group by `status == "orphaned"` if you want the historical
  matched-vs-orphaned partition. Aligns `query` with `list` so consumers
  handle one shape across both. The new `update`, `delete`, and `prune`
  subcommands' payloads (`updateData`, `deleteData`, `pruneData`) join
  the contract under the same versioned envelope.
- **Breaking (UX):** `ynotes init` is now idempotent — a second invocation
  in a directory that already has a store reports `ynotes store already
  exists at <path>` on stdout and exits `0`, instead of exiting `2` with
  a refusal. This follows the "already in the desired state → success"
  pattern, so an agent that pattern-matches to other Unix CLIs reads the
  right outcome. The store is still
  refused — as `Error::Invalid` — when `.ynotes` exists but lacks a `HEAD`
  sentinel, since adopting an unrelated directory of that name would be
  unsafe. `Error::StoreExists` is removed from the library `Error` enum
  (no caller branches on it any more).
- **Breaking (agent contract):** the `--json` schema is bumped to `"v": 4`.
  Every payload now rides inside a uniform envelope —
  `{success: true, v: 4, data: <payload>}` on success;
  `{success: false, v: 4, error: "<message>", type: "<kind>"}` on
  failure — and the envelope is emitted on stdout regardless of exit
  code. A JSON consumer branches on the `success` field instead of
  checking `$?` before parsing; `type` is a stable enum (`usage`,
  `engine`, `io`, `render`) so a consumer can react to the error class
  without regex on the message. The envelope shape is `{success, data}` on
  success / `{success, error}` on failure. Existing consumers move from
  `.matched[]` to `.data.matched[]`, etc. — every existing key kept its
  name; one nesting level is the only change beyond `success`/`v`/`type`.
- **Breaking (agent contract):** the `--json` schema is bumped to `"v": 3`.
  - `query --json` and `list --json` gain an always-present `warnings: []`
    array. `query --json` populates it when the target file is missing
    (almost always a typo); `list --json` populates it when a path filter
    matched nothing — distinguishing "this filter is empty" from "the store
    is empty" without reparsing stderr. With no filter, an empty `notes`
    array is self-explanatory and `warnings` stays empty. Exit code stays
    `0`; the warning is the in-band signal.
  - The `scope` field collapses `"line"` into `"range"`: a single-line note
    is now `{"scope":"range","range":[n,n]}`. Consumers handle one shape for
    a contiguous region; intent (was-it-typed-as-a-single-line) is
    recoverable from `range[0] == range[1]`. The internal `Scope::Line`
    enum is preserved so existing note ids — content-hashed over
    `(target, scope, bundle, body)`, invariant #6 — stay valid.

### Fixed

- A path that resolves outside the work tree now consistently exits `2`
  (usage error) for `save`, `query`, and `list` — matching the existing
  symlink-escape guard's exit code. Previously the absolute-path form
  routed through `Error::Invalid` and exited `1` (engine error) while the
  symlink-escape form exited `2`; same user mistake, two different exit
  codes. A new typed `Error::OutsideStore { path, store_root }` variant
  (the engine enum is `#[non_exhaustive]`, so additive) is the single
  source of classification — the binary's `From<ynotes::Error>` maps it
  to `CommandError::Usage`. Field-reported by an external review.
- `Store::save` no longer over-reports `created: true` under contention.
  The check-then-write TOCTOU window let two concurrent identical saves
  both see "file absent" and both write, both reporting `created: true`
  even though only one record physically existed. The persistence step is
  now `tempfile::NamedTempFile::persist_noclobber` (POSIX `link`+`unlink`,
  Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`), which is
  atomic: the loser of a race gets a clean `Ok(false)`. A new integration
  test runs eight parallel saves of identical content and asserts exactly
  one `created: true`.
- The empty-body `save` error names the actual cause rather than the
  symptom: `"no note body provided — pass \`-m "<text>"\` or pipe text on
  stdin"`, replacing the prior `"refusing to save an empty note (use -m
  or pipe text on stdin)"`. A field reviewer mistook the old message for
  a `-m` + stdin conflict; the new wording forecloses that misreading.
- `json_envelope::wrap` no longer double-emits when a command body
  signals a non-zero exit alongside an already-printed *success* envelope
  (the new `delete` pattern: per-id outcomes ride in the success envelope;
  the exit code carries "some ids failed"). `Rendered` errors are now
  passed through unchanged instead of triggering a second `print_error`
  call — stdout carried two JSON documents back-to-back before the fix.
- `ynotes save --json` is serialised with `serde_json` rather than string
  interpolation, so a target path containing a character that JSON escapes
  (e.g. a quote) can no longer produce malformed output. The `--json` surface
  is an agent contract and must always parse.
- A location argument of `0` is rejected as a usage error everywhere. Line
  numbers are 1-based, so `0:5` and `save … 0` already failed; a bare
  `query <file> 0` now does too, instead of silently behaving as a
  whole-file query.
- A note's target path is derived from the file path's components rather than
  a blanket `\` → `/` rewrite. A backslash is a path separator only on
  Windows; on Unix it is an ordinary filename character, so the old rewrite
  could store a note on `a\b.rs` under the wrong key and leave it permanently
  orphaned in `list`.
- `reanchor` against a dirty working tree no longer desyncs the git-transport
  rung. The git selector now stores the region's range in its baseline
  commit's own coordinates — captured by translating the working-tree range
  back through the commit diff — so the `(commit, range)` pair stays
  internally consistent, and a later `query`, or a later commit of the
  pending edit, transports it with no offset. Previously a `reanchor` run on
  an edited-but-uncommitted tree (the documented workflow) baked a permanent
  line skew, equal to that uncommitted edit, into every refreshed note's git
  rung; the same flaw affected a `save` against a dirty tree. The `git`
  selector gains an optional `range` field — a note written before it falls
  back to the position selector, so existing stores keep loading.
- A region whose enclosing construct has been deleted now resolves `orphaned`
  rather than `drifted` at high confidence. Resolution counts only the
  content rungs (exact quote, structural, fuzzy) as evidence the region still
  exists: git transporting a deleted range onto the deletion point, and the
  position rung flooring into a now-shorter file, are no longer mistaken for
  a relocation. Previously a fully-deleted region was re-pointed onto
  unrelated code and stamped `confidence: 100`.
- `query` and `list` text output no longer prints `drifted ⚠ was X` when X is
  the range the note already resolved to. A content-only drift — an in-place
  edit, or any note just refreshed by `reanchor` — keeps its saved position,
  so the `was …` clause now appears only when the region genuinely moved.
- The re-anchoring combiner no longer lets the `git` and `structural` rungs
  out-vote the content rungs. Field testing found it false-anchoring a
  deleted region onto unrelated code (resolving `drifted`) and reporting an
  edited region as `anchored` — the engine's core guarantee inverted. The
  combiner is now a decision procedure, not a weighted score: only an exact
  `quote` match, a `fuzzy` match, or a `structural` match with an identical
  content fingerprint establishes that a region's code is present; `git` and
  `position` corroborate but never decide. A region no content rung locates
  resolves `orphaned`; `anchored` again means located *and* intact.
- The `structural` rung's fingerprint now includes identifier and literal
  text, not only node kinds, so a renamed local or a changed literal is
  visible to it. Previously a structure-preserving edit kept a byte-identical
  fingerprint and the rung reported the region intact at full score.
- **Removed** the per-note `confidence` integer from the `save`/`query`/`list`
  `--json` output; the schema version `v` is now `2`. The combined 0–100
  score was fabricated — at times higher than any rung it was derived from —
  and an agent branches on the `status` category, not a numeric threshold.
  `status` and the derived `stale` boolean are the trust signal; per-rung
  scores remain under `query --explain`. `reanchor` correspondingly refreshes
  any non-orphaned note rather than gating on a confidence threshold.
- The fuzzy rung (R3) now recognises a region that was edited in place — for
  example with every local variable renamed — instead of orphaning it. Lines
  are matched by character similarity rather than exact equality, and a region
  with no exact-surviving line is relocated by scoring the saved block against
  a window of offsets around its last known position. Such a region is now
  reported `drifted`, not `orphaned`.
- `save` no longer attaches a git selector for a file that is not tracked in
  `HEAD`. An untracked file has no baseline commit, so the git-transport rung
  could previously report a spurious result for it; such a file now captures
  `git: None`, leaving R1 to report `skipped` while the content rungs carry
  resolution.
- `save` now genuinely supersedes. Re-saving a note at the same location
  (same target, scope, and original line range) with an edited body retires
  the earlier note instead of leaving both live, so `query` no longer returns
  a stale and a current copy of the same context. The new note is written
  before the old one is removed, so a crash mid-operation cannot lose a note;
  an identical re-save remains a pure no-op.
- `save` reading the body from stdin no longer keeps the pipe's trailing
  newline(s): a piped body and a `-m` body now store identical text, and so
  share a content-addressed id for identical content.
- `save <file> <line-past-end-of-file>` now exits `2` (a usage error), to
  match `save <file> 0`; previously a beyond-EOF range exited `1` as a
  runtime error.
- `reanchor` no longer prints a self-contradictory `X:Y -> X:Y` line for a
  note whose selectors were refreshed but whose line range did not move; it
  now reads `<target> X:Y (selectors updated, range unchanged)`.
- `query` on a path that does not exist and has no notes now reports that the
  file does not exist, on stderr, instead of the generic "no notes for that
  query" — so a mistyped path fails loudly. The exit code stays `0`: a query
  that finds nothing is not an error.
- The resolver no longer re-anchors a note onto a same-shaped sibling when
  the original region was deleted. Field testing found a note saved on one
  function re-pointing onto a sibling after the former was removed — both
  shared their scaffolding lines, so the fuzzy rung's LCS landed on the
  sibling's body, no other rung agreed, and `reanchor` then welded the note
  there at `status: anchored`. The combiner now treats a fuzzy hit as
  sufficient only when (a) fuzzy resolved to the *exact* saved range — its
  position-window scoring picked the saved offset over every alternative,
  the rung's own evidence that the region did not move — or (b) git
  transport or the structural rung (at any score) independently points at
  the same range. Otherwise the note resolves `orphaned` and `reanchor`
  leaves it alone for a human; the position rung is not consulted here,
  since its range is *the saved range*, and treating that overlap as
  corroboration is circular. Invariant #4 over a confidently-wrong
  attribution.
- `save` and `reanchor` now refuse a target whose canonical path lies
  outside the work tree. The lexical `relativize` (which must work for files
  that do not yet exist) let a symlink like `inside.lnk -> /etc/passwd`
  through at save time, and a real file whose path was later swapped for
  such a symlink would have been picked up at the next `reanchor`. In
  either case the external content would have been hashed into a
  selector bundle and — if `.ynotes` is committed — leaked downstream.
  Both commands now canonicalise the target and the work tree and reject
  the escape (`save` exits with a usage error; `reanchor` skips the note
  with a named reason). Intra-repo symlinks (target resolves back into the
  work tree) are unaffected.
- `list <dir>` now filters by prefix instead of silently reporting "no notes
  in this store" while the store has notes elsewhere. The boundary uses a
  trailing `/`, so `list sa/` does not match `saas/...`. The empty-result
  message distinguishes "no notes under `<dir>`" from "no notes in this
  store".

[Unreleased]: https://github.com/ars-2107/ynotes/commits/main
