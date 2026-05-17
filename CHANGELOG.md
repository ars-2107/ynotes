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
  `cargo-nextest` + `insta` testing, a `justfile`, CI (format, clippy,
  cross-platform tests, MSRV, supply-chain audit, `zizmor` Actions analysis),
  and cargo-dist release configuration.
- `AGENTS.md` — the canonical agent guide (architecture, error strategy,
  comment doctrine, commands, code style, documentation-propagation checklist).
  `CLAUDE.md` is a symlink to it: both filenames work, one source of truth, no
  drift.

[Unreleased]: https://github.com/ars-2107/ynotes/commits/main
