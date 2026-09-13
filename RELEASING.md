# Releasing ynotes

Releases publish to GitHub Releases and npm, not crates.io.
Pushing a `v*` tag starts publication. Do not push one to test the workflow.

## Prepare

1. Choose an unused version. Check remote tags, GitHub releases, and the
   launcher plus all seven platform packages on npm.
2. Set the workspace version in `Cargo.toml`, then run `just sync-version`.
   Do not edit generated npm or plugin versions by hand.
3. Give the release a dated `## [X.Y.Z]` section in `CHANGELOG.md`.
   The workflow extracts this section as release notes.
4. Confirm that the GitHub `release` environment has `NPM_TOKEN`, and that
   the publishing account controls all eight package names.

## Validate

Run on the candidate revision:

```sh
just ci
cargo +1.88.0 check --locked --all-targets --all-features
cargo build --release --locked --bin ynotes
node scripts/smoke-install.mjs target/release/ynotes
```

The smoke runner packs the launcher and the host's native package, checks
MIT metadata and licence files, installs offline in a temporary prefix, and
tests discovery, save/query, renames, the session hook, reanchor, extraction
recovery, guidance, and doctor. It prints the retained artefact directory
and does not change a global installation.

The local runner supports macOS and Linux. Check the complete CI matrix on
the exact release revision, including Windows, Alpine/musl, MSRV, and workflow
analysis. Inspect archive contents and licence files before publication.

## Publish and verify

After maintainer approval, tag the reviewed commit `vX.Y.Z` and push the tag.
The tag must match `Cargo.toml`; do not reuse or move an existing release tag.

The workflow builds seven targets, uploads archives and `SHA256SUMS` to GitHub,
and publishes platform packages before the npm launcher. Check all jobs through
completion.

Install the released version from npm in a fresh temporary prefix. Check its
version, doctor, save/query, and MCP handshake. Verify the release archives and
checksums against that version. A successful build alone is not release proof.
