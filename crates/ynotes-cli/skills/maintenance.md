---
name: maintenance
description: Maintain ynotes stores. Use after refactors, for drifted or orphaned notes, when doctor reports record or index problems, or for an explicitly requested cleanup.
allowed-tools: Bash(ynotes:*)
---

# Store maintenance

Reads do not change records or repair the index. Preview write operations and
inspect their results before applying them.

## Reanchor

After a refactor, file move, or merge:

```sh
ynotes reanchor --dry-run
ynotes reanchor
```

The report separates:

- `changed`: refreshed selectors, with new IDs when identity changed.
- `relocated`: followed a committed file rename.
- `unchanged`: no selector update needed.
- `skipped`: left untouched; `reason_code` explains why.
- `review_pending`: location already refreshed, but current code still differs
  from the note's review basis. These entries carry current IDs.

Changed and relocated entries include `review_required`. Reanchor preserves
the review basis and does not clear it on a second pass.

Orphans are never reanchored. Staged-only renames are deferred until committed;
reads can surface the note at the destination meanwhile.

## Review drifted notes

1. Read `ynotes show <id>` and the current code, including dependencies relevant
   to the claim.
2. If only the location moved, reanchor can persist it without confirmation.
3. If code changed but the claim still holds, reanchor if needed, then confirm
   using the current ID.
4. If the claim needs correction, use `ynotes update <id> -m "corrected context"`.
5. Delete the note only if its claim is wrong or obsolete.

Confirm refuses orphans and regions located only by similarity. Inspect a
locatable fuzzy match before reanchor and confirm; neither command substitutes
for checking identity and claim accuracy.

## Inspect orphaned notes

An orphan means the region cannot be identified, not necessarily that its
concept was deleted.

Search for the identifiers and inspect Git history. If the code moved into
another existing file, review and save the note at that destination with
`--code`. Query to verify the replacement before deleting the old record.
An identical quote alone is not proof of identity.

Keep useful historical context. Use `ynotes prune --dry-run` only for an
explicitly requested cleanup, and inspect every proposed removal before applying.

## Store health

Run `ynotes doctor --json`.

| Finding | Action |
| --- | --- |
| Malformed records | Inspect the reported files before repair or deletion. |
| Missing or malformed index, or nonzero drift counts | Preview and run `ynotes reindex`. |
| Missing merge guards | Store writes attempt repair; rerun doctor and fix the file manually if still missing. |

`.ynotes/notes/` is authoritative. The index is a cache. Reads recover omitted
records without writing; mutations require a valid index.
Reindex rebuilds it without changing note contents or IDs, but cannot fix a
malformed note record.

A corrupt canonical record requires its exact 64-character ID for deletion.
Prefixes are refused because the body cannot be previewed. Irregular unindexed
files may report null identity fields; inspect their reported `path` manually.
Successful purges appear in `deleted_unreadable[]`.

## Debug an anchor

`ynotes show <id> --explain` or `ynotes query <file> --explain` reports each
matching method's outcome and diagnostic score. See
`ynotes skills get core --full` for the matching reference.
