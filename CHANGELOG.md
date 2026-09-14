# Changelog

## [Unreleased]

- `list`, `files` and `reanchor` now follow a file rename commit by commit
  from the note's baseline, and compose multi-hop moves, instead of one diff
  across the whole span. A file edited heavily and then renamed no longer
  reads as orphaned in those commands while `query` on the new path finds it.

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
