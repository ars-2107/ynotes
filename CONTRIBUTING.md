# Contributing to ynotes

Read [AGENTS.md](AGENTS.md) for architecture, data-safety rules, and API
contracts before changing code.

## Setup and checks

Use Rust 1.88 or newer. Install `just`, `taplo`, `cargo-nextest`, and
`cargo-deny` to run the local checks:

```sh
just
just ci
cargo +1.88.0 check --locked --all-targets --all-features
```

`just ci` checks formatting, Clippy, API docs, tests, dependency advisories
and licences, and manifest versions. CI also checks supported platforms,
the minimum Rust version, package installation, and GitHub Actions workflows.
A local pass does not replace those jobs.

Add regression tests for behaviour changes. Do not skip failing tests or
weaken assertions to make a change pass. Snapshot tests use
[insta](https://insta.rs); review each changed snapshot before accepting it.
Doctests run separately from nextest.

## Code and documentation

Keep behaviour in `crates/ynotes-core`. The CLI and MCP crates handle input,
protocols, and presentation. The core stays synchronous and forbids unsafe code.

Document public APIs and non-obvious constraints. Include `# Errors` and
`# Panics` where applicable, and testable examples for non-trivial APIs.
Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).
Comments should explain why the code needs a constraint, not repeat the code
or recount a development session.

Use British English and no em dashes. Keep release history in
[CHANGELOG.md](CHANGELOG.md), not in source comments or usage guides.
User-facing changes also need updates to help text, embedded agent guidance,
and the schema when its shape changes.

## Pull requests

Describe the behaviour change, its reason, and the checks you ran.
Keep unrelated changes separate. Use
[Conventional Commits](https://www.conventionalcommits.org/) with a short,
imperative subject.

Contributions are under the project's [MIT licence](LICENSE).
