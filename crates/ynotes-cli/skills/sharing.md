---
name: sharing
description: Share a ynotes store through Git. Use when committing .ynotes, resolving store merge conflicts, recovering notes after merges, or setting team and multi-agent conventions.
allowed-tools: Bash(ynotes:*)
---

# Sharing notes

Commit `.ynotes/` to share notes through Git. Records contain note bodies and
code excerpts. Review them for secrets and sensitive content before committing.

## Store format

Each note is an immutable JSON file named by its content hash.
The index is a rebuildable JSONL cache. Managed `.ynotes/.gitattributes` rules
use Git's built-in merge handling:

- `index/by-path.json merge=union` preserves index entries from both sides.
- `notes/** -merge` prevents conflict markers from being inserted into note JSON.

Store writes try to restore missing guards. This repair is best-effort;
`ynotes doctor` reports guards that still need attention.

## Merges

Independent additions usually touch different record files. Concurrent updates,
deletions, or reanchors of the same note can still conflict.

Inspect `git status` and both versions of each conflicting record. Preserve
distinct note bodies before staging a resolution; blindly staging the working
copy does not guarantee both versions survive. Do not hand-edit a record without
also preserving its content-addressed identity. When needed, retain copies
outside the store and recreate reviewed notes with the CLI after the merge.

Do not run reindex mid-merge because it complicates `git merge --abort`.
After resolving the merge:

```sh
ynotes doctor
ynotes reindex --dry-run
ynotes reindex
ynotes reanchor --dry-run
```

Apply reanchor after reviewing its report. Identical notes whose refreshed
identity matches can collapse; distinct bodies remain separate and need review.

## Team conventions

- Review note diffs like source comments. A note's claim may be wrong even when
  its code remains anchored.
- Write standalone explanations without session references or progress logs.
- Keep useful orphaned context. Bulk pruning needs an explicit cleanup decision.
- Query notes after merging and review any changed claims.

## Recovery

`ynotes reindex` rebuilds the index from record files without changing their
contents or IDs. It cannot repair a malformed record. Inspect such records
before using `ynotes delete <full-id>`; an irregular unindexed file may have no
usable ID and needs manual inspection.

Use `ynotes skills get maintenance` for health findings and review procedures.
