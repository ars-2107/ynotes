## Command reference

Every command also takes `-v`/`-vv`/`-vvv` (log verbosity, stderr; the
`YNOTES_LOG` env var overrides it). Diagnostics go to stderr; stdout
carries only data.

| command | does | key flags |
|---|---|---|
| `init` | create or repair a `.ynotes` store in the current directory | `--json` |
| `save <file> [LINE\|START:END]` | save context for a file or region | `--code <text>` verifies the location; `-m <text>` body (stdin if omitted); `--json` |
| `query <file> [LINE\|START:END]` | return the context overlapping a region | `--json`; `--count` breakdown only; `--explain` per-rung outcomes |
| `files` | annotated paths and counts; missing targets get rename corroboration | `--json` |
| `list [file]` | every note in the store (or one file), with status | `--json`, `--count`, `--explain` |
| `show <id>` | one note by id or unambiguous hex prefix, resolved against current code | `--json`, `--explain` |
| `lookup` | find ids by a stable handle | `--target <path>`, `--body-contains <text>` (at least one), `--json` |
| `update <id>` | replace a note's body in place | `-m <text>` (stdin if omitted), `--json` |
| `confirm <id>` | mark a note body reviewed against current code; refuses any region the ladder cannot identify (orphan, or similarity-only) | `--json` |
| `delete <id>…` | delete notes by id or unambiguous prefix (≥4 chars) | `--dry-run`, `--json` |
| `prune` | remove every orphaned note | `--dry-run`, `--json` |
| `reanchor` | persist re-anchors for notes the ladder located; orphans, missing targets, and symlink escapes are skipped | `--dry-run`, `--json` |
| `reindex` | rebuild the by-path index from the notes on disk | `--dry-run`, `--json` |
| `doctor` | environment and store health, read-only | `--json` |
| `skills` | this guidance: `list`, `get <name>`, `get core --full` | |
| `completions <shell>` | shell completion script | |
| `mcp` | the MCP stdio server (spawned by agent clients, not interactive) | |

Every MCP tool requires `root`, an absolute repository path. Relative file
arguments resolve beneath it; MCP never uses the server cwd for selection.

Exit codes: `0` success (empty results included), `2` usage (bad
invocation, an invalid id prefix, a path outside the store), `1` real
failure, plus `delete`'s partial-failure case (success envelope, exit 1).

## Payload shapes (contract v1)

The full JSON Schema ships as `ynotes.schema.json` in the repository; the
MCP tools emit payloads from the same definitions. The shapes an agent
branches on:

- **envelope**, success `{"success": true, "v": 1, "data": …}`; failure
  `{"success": false, "v": 1, "error": "…", "type": "usage"|"engine"|"io"|"render"}`.
- **note** (in `query`/`list`/`show` data), `id`, `target`, `scope`,
  `resolved_range` (`[start, end]`, or `null` for an orphan, there is no
  `range` field on a note), `previous_range` (present when the region is not
  at its saved range: a moved `anchored` or `drifted` note, or an `orphaned` one), `body`, `status`
  (`anchored`|`drifted`|`orphaned`), `stale` (boolean, `false` only for
  `anchored`), optional `relocated_from` and `relocated_to` (the stored and
  current sides of a pending rename), and `rungs` under `--explain`.
- **initData**, absolute store `root`, and `created`.
- **saveData**, `id`, `target`, `scope`, `range`, `created` (`false` on
  an idempotent re-save), `store_created` (`true` when the save
  bootstrapped the store), and `neighbours[] {id, range, body_excerpt}`,
  the other notes already on that file, so a near-duplicate does not land
  unnoticed.
- **updateData**, `id`, `previous_id`, `target`, `scope`, `range`,
  `created`.
- **confirmData**, `id`, `previous_id`, `target`, `scope`, `range`,
  `changed`, `created`.
- **deleteData**, partitions `requested`, `deleted`, `deleted_unreadable`,
  `ambiguous`, `not_found`, plus `dry_run`.
- **filesData**, `files[] {target, notes}` (most-annotated first), `total`,
  `malformed`, `index`. Missing targets follow corroborated Git renames;
  unlocatable notes retain their historical path. No ranges or statuses.
- **storeAbsent**, `{"store":"absent"}` when CLI `files` or an MCP read
  finds no store. Success, not an instruction to initialise one.
- **reanchorData**, partitions `changed`, `relocated`, `skipped`,
  `review_pending`, `unchanged`, plus `dry_run`.
- **count data** (`query --count` / `list --count`), `{total, anchored,
  drifted, orphaned}` plus a `malformed` count and index health.
- **index health**, `{status, recovered, dangling}` on query/list and their
  count variants. `status` is `healthy`, `missing`, or `malformed`; reads
  recover from `notes/` but never repair the cache.
- **malformed[]**, always present on `query`/`list` data: `{id, target,
  path, error}` per unreadable record. `id`/`target` may be null when an
  unindexed record cannot prove them.
- **doctorData**, `version`, `platform`, `git`, `store`, and a `health`
  block: `malformed`, `index` health (`status`/`recovered`/`dangling`), and
  `gitattributes_missing`.

## Id semantics

A note id is a content hash of `(target, scope, bundle, review_basis, body)`, never a
timestamp. Consequences: `save` is idempotent while the working state is
unchanged (a retry returns the same id, `created: false`; after a commit
the bundle's git baseline has changed, so the same save creates a fresh
note); ids rotate on `update`, on `confirm` when the review basis advances,
and on `reanchor` when the bundle changes; two copies of one
note left behind by a concurrent merge collapse into one when they
re-anchor to the same place. Cache `(target, body)` and re-resolve with
`lookup`, not the id.

## The anchor ladder (--explain)

Resolution runs independent rungs and reports each outcome with a
diagnostic score in [0, 100]. There is no combined confidence number, the
verdict is the categorical `status`:

| rung | score | meaning |
|---|---|---|
| `git` | 100 | range unchanged in the working tree |
| `git` | 90 | git transported the range; the touched lines did not overlap it |
| `git` | 70 | transported, but the moved range was itself edited |
| `quote` | 95 | unique exact text at the saved position, identified by context or capture-time uniqueness |
| `quote` | 90 | prefix/suffix context disambiguated, or the quote was unique at capture and is unique now |
| `quote` | 55 | several occurrences, none disambiguated, nearest to the saved line chosen |
| `structural` | 95 | enclosing construct found, content fingerprint identical |
| `structural` | 55–94 | construct found, fingerprint drifted (linear in similarity) |
| `fuzzy` | 45–80 | line-LCS alignment above the threshold; capped below 90, a fuzzy match alone is never `anchored` |
| `position` | 20 / 10 | the saved range still fits / was clamped to fit; diagnostic only, never consulted by the verdict |

`result: "miss"` means the rung ran and found nothing. `result: "skipped"`
means it could not run (no git repository; an unsupported language for
`structural`) and is not held against the verdict.

## Environment

- `YNOTES_DIR`, points directly at a `.ynotes` directory, overriding
  discovery. Errors if it is not a valid store. **CLI only**: MCP tools
  select their store from the `root` argument and never consult it, so
  setting it cannot redirect an agent's writes.
- `YNOTES_LOG`, tracing filter for stderr diagnostics; overrides `-v`.
- `NO_COLOR`, honoured; output is plain text regardless.
