# ynotes

> Store code context that survives edits, and hand an agent only the context it asks for.

**Status:** early scaffold. The architecture, tooling, and conventions are in
place; the engine itself is not yet implemented. This is a production-grade
Rust starter — see [`AGENTS.md`](AGENTS.md) for the design and where each piece
will go.

## What this is

ynotes is a git-like CLI for attaching context to regions of code. Unlike
comments, the context is not carried inside the file (so it costs an agent no
tokens until requested); unlike a notes vault, it is anchored to the code and
re-anchors itself when the code moves. Retrieval is range-based: ask for any
line range and ynotes returns the context overlapping it.

The hard problem — re-anchoring context across edits without it going silently
stale — is deliberately *not* solved in this scaffold. The scaffold exists so
that problem can be solved on solid foundations.

## Layout

One crate, two faces (the `bat` model):

```
src/lib.rs        the engine — all behaviour lives here (the library)
src/main.rs       a thin binary: parse args, dispatch, exit code
src/cli.rs …      binary-only modules (not part of the library)
```

A library boundary *inside* one crate keeps the start lean (the shape Vercel's
own Rust CLIs use) while making the eventual extraction into a `ynotes-core`
crate — the day an MCP server needs the engine too — a mechanical move rather
than a rewrite. Full rationale in [`AGENTS.md`](AGENTS.md).

## Build and run

Requires a Rust toolchain (see `rust-toolchain.toml`; MSRV 1.85).

```sh
cargo run --bin ynotes -- doctor
cargo run --bin ynotes -- completions zsh
```

With [`just`](https://github.com/casey/just): `just` lists every task;
`just ci` runs the exact gate CI enforces (format, lint, test, audit). Tests
use [`cargo-nextest`](https://nexte.st); releases use
[cargo-dist](https://opensource.axo.dev/cargo-dist/) (`dist init`).

## License

Dual-licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT)
at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
