# Task runner for ynotes. Install `just` (https://github.com/casey/just), then
# run `just` for the list. CI runs exactly `just ci`, so green locally ⇒ green
# in CI.

# Show all recipes.
default:
    @just --list

# Format Rust and TOML.
fmt:
    cargo fmt --all
    taplo fmt

# Verify formatting without writing (what CI runs).
fmt-check:
    cargo fmt --all -- --check
    taplo fmt --check

# Lint with warnings promoted to errors, across every target.
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Fast type-check, no codegen.
check:
    cargo check --all-targets --all-features

# Build the API docs; a broken intra-doc link fails the build (AGENTS.md).
docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# Tests via nextest, plus doctests (nextest does not run doctests).
test:
    cargo nextest run --all-features
    cargo test --all-features --doc

# Supply-chain audit (advisories, licences, bans, sources).
deny:
    cargo deny check

# Verify the npm manifests' versions match Cargo.toml (CI enforces this).
version-check:
    node scripts/sync-version.mjs --check

# Static analysis of the GitHub Actions workflows (injection, misconfig).
audit-ci:
    pipx run zizmor .

# The exact gate CI enforces, in order. Run before pushing.
ci: fmt-check lint docs test deny version-check

# Optimised release build.
build:
    cargo build --release --bin ynotes

# Run the binary. Example: `just run -- doctor`
run *ARGS:
    cargo run --bin ynotes -- {{ARGS}}

# Propagate the Cargo.toml version into the npm manifests (package.json and
# npm/*/package.json). Run after bumping the version, before tagging a release.
sync-version:
    node scripts/sync-version.mjs
