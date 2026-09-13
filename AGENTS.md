# Repository instructions

## Architecture

ynotes attaches context to code regions and resolves those regions after edits.

- `crates/ynotes-core` owns behaviour. Its library target is named `ynotes`.
- `crates/ynotes-cli` parses arguments, dispatches calls, and renders results.
- `crates/ynotes-mcp` handles MCP stdio. It calls the synchronous core through
  `spawn_blocking`; keep Tokio out of the core.

New CLI commands belong in `cli.rs::Command`, `commands/`, and dispatch.
New MCP tools belong in their own module and register on the server. Use the
shared contract payloads, `ok_json`, and schemas from `output_schema.rs`.
Neither front-end should implement engine policy.

## Development

```sh
just
just ci
cargo +1.88.0 check --locked --all-targets --all-features
cargo run --bin ynotes -- doctor
```

Run `just ci` before committing or merging. It checks formatting, Clippy,
docs, nextest, doctests, dependency policy, and version synchronisation.
CI additionally checks MSRV, platform installation, and workflows.
Do not skip failing tests, suppress failures, or narrow coverage to pass.
Review intentional snapshot changes with `cargo insta review`.

The library returns typed `ynotes::Error` variants. CLI `CommandError` adds
presentation and exit codes: 2 for usage errors, 1 for other failures.
Quote-verified saves are exact-or-refuse, using `CodeNotFound` and
`CodeAmbiguous`. Fuzzy matching belongs only in read-time resolution.

Use kebab-case flags, British English, and no em dashes or emojis.
Unicode status marks are allowed. Send diagnostics to stderr through
`tracing`; reserve stdout for data. Honour `NO_COLOR`; do not hardcode ANSI.

## Documentation and comments

Open modules with their responsibility and relevant constraints. Document public
items, including `# Errors`, `# Panics`, and compiled examples where applicable.
Keep inline comments for non-obvious reasons, compatibility, or safety.
Do not add session history, progress reports, or claims that tests prove more
than they exercise. Broken intra-doc links fail the build.

For user-visible changes, update the affected:

- CLI help and MCP descriptions, including the reviewed `tools/list` snapshot.
- README examples and `CHANGELOG.md` under `[Unreleased]`.
- Public API docs and these instructions if a rule changes.
- `ynotes.schema.json` and contract snapshots if payload shapes change.
- Embedded guidance in `crates/ynotes-cli/skills/`.
- Plugin stub in `skills/ynotes/SKILL.md` and `.claude-plugin/`.
- `hooks/hooks.json` and `commands/hook.rs` for harness behaviour.

The core skill and MCP server instructions must describe the same workflow.
Keep the plugin stub pointing to installed guidance rather than duplicating
tool lists. Preserve the session hook's silence for absent stores, empty stores,
and unreadable events.

## Invariants

Keep these numbers stable because source comments refer to them.

1. No unsafe code.
2. Behaviour belongs in the core, not either front-end.
3. The core is synchronous. Async belongs only in MCP.
4. Never silently drop user-authored context. Return unlocatable notes as
   `orphaned`, and report malformed records separately.
5. MSRV is 1.88. Keep `Cargo.toml`, `clippy.toml`, and the CI job in sync.
6. Note IDs hash `(target, scope, bundle, review_basis, body)`.
   Time and randomness must not affect identity or retry idempotence.
7. JSON is a versioned contract, currently v1. Field changes require a version
   bump, the published schema, and `agent_contract.rs` snapshots. CLI and MCP
   share payload definitions and both have schema-conformance tests.
8. Resolving notes never writes. Only explicit `reanchor` persists resolution;
   it refuses orphaned notes. Keep read paths usable on read-only stores.
9. `Cargo.toml` owns the version. `scripts/sync-version.mjs` propagates it to
   npm and plugin manifests; do not hand-edit generated versions.
10. Git rename detection only proposes a destination. Content matching must
    corroborate the region before any read surfaces it there or reanchor moves
    it. Apply the same guard to inventory and pruning.
11. Selector maintenance preserves `review_basis`. Only save, update, or
    explicit confirm may advance it. Changed code remains `drifted` until
    reviewed. Confirm requires identified code, not an orphan or fuzzy-only
    guess. Reanchor reports `review_required` and `review_pending`.
12. Note records are authoritative; the index is a cache. Reads recover records
    omitted by a missing, malformed, or incomplete index without repairing it.
    Mutations other than reindex validate the index before changing records.
13. Every MCP call requires an absolute `root`. Relative file paths resolve
    beneath it. Do not infer a root from cwd, walk to a parent store, or apply
    the CLI's `YNOTES_DIR` discovery override.
14. Exact-quote identity includes capture-time uniqueness. A surviving duplicate
    does not become the original merely because its sibling disappeared.
    Legacy tracked notes reconstruct uniqueness from their Git baseline when
    available. Saved position and fuzzy matching cannot override ambiguous
    identity. Preserve canonical-path and symlink confinement on writes.
15. Anchor status is not claim truth. Unchanged local code does not validate
    assumptions about callers, configuration, or external constraints.
16. `files` checks existing targets cheaply and follows corroborated renames
    for missing targets without writes. Unlocatable notes retain historical
    paths. An absent store is successful discovery, not automatic adoption.

## Releases

Follow [RELEASING.md](RELEASING.md). Publishing requires explicit maintainer
approval. A tag starts public GitHub and npm publication; it is not a test.

Keep the handwritten release workflow. Do not introduce cargo-dist, add
`cargo publish`, or remove `publish = false` without an explicit decision.
Distribution uses a thin npm launcher and seven platform packages, plus GitHub
archives. All project packages declare MIT and include the root `LICENSE`.
Dependency licence allowances in `deny.toml` are separate from project licensing.
