# Changelog

## [Unreleased]

- `list`, `files` and `reanchor` now follow a file rename commit by commit
  from the note's baseline, and compose multi-hop moves, instead of one diff
  across the whole span. A file edited heavily and then renamed no longer
  reads as orphaned in those commands while `query` on the new path finds it.
- The session hook adds one line naming how many notes are drifted or
  orphaned and pointing at `ynotes reanchor --dry-run`, for stores of up to
  500 notes; larger stores keep the file list only.
- A region whose text is intact but moved within its file now resolves
  `anchored` at its new position, with `previous_range` reporting where it
  was; `drifted` now means the content changed. `stale` follows `status`, so
  a pure move no longer asks the reader to re-verify unchanged code. The text
  renderer shows `anchored was A:B` for a moved region.
- A note anchors to its construct, not to its lines. When an edit separates
  a construct's first line from the rest of a noted region, the note now
  stays on the construct the structural rung still names, reported
  `drifted`, instead of following the body lines to wherever they moved,
  even into another function.

## [0.1.0]

Initial release candidate.

- Local code notes with file and line-range queries.
- Git, text, and syntax-tree matching with explicit anchor status.
- Review confirmation, rename handling, and store maintenance.
- CLI, MCP server, and Claude Code plugin.
- JSON contract v1 and selector format v1.
- npm and standalone binary distribution.
- Rust 1.88 minimum. MIT licensed.

[Unreleased]: https://github.com/ars-2107/ynotes/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ars-2107/ynotes/releases/tag/v0.1.0
