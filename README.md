# ynotes

Attach notes to code and retrieve them by file or line range.

ynotes stores notes in `.ynotes/`, separate from source files. It uses Git,
exact text, fuzzy matching, and syntax trees to find the code after edits.
If it cannot locate a region, it returns the note as `orphaned`.

Use it for constraints and decisions that are not clear from the code.
Notes are local files that you can review and commit with your repository.
No account, hosted service, or model API is required.

Pre-1.0. APIs and storage formats may change between minor versions.
See the [changelog](CHANGELOG.md) for compatibility changes.

## Install

Build from source with Rust 1.88 or newer:

```sh
git clone https://github.com/ars-2107/ynotes.git
cd ynotes
cargo build --release --locked --bin ynotes
export PATH="$PWD/target/release:$PATH"
ynotes doctor
```

The release workflow distributes binaries through npm and
[GitHub Releases](https://github.com/ars-2107/ynotes/releases).
Once a release is published, install it with `npm install -g ynotes`.
The npm launcher requires Node.js 18 or newer. There is no crates.io package.

## Use

Suppose `src/retry.rs` contains this at lines 42 through 44:

```rust
fn retry_delay_ms() -> u64 {
    100
}
```

Save the constraint behind it:

```sh
ynotes save src/retry.rs 42:44 \
  --code 'fn retry_delay_ms() -> u64 {' \
  -m 'Keep this delay below the caller timeout of 250 ms.'
ynotes query src/retry.rs 43
```

Line numbers are 1-based and ranges include both ends. `--code` verifies the
start of the region and corrects stale coordinates when it finds a unique
exact match. If the text is missing or ambiguous, reread the file and refine
the quote or coordinates.

The first save creates a store at the enclosing Git root. Outside Git, run
`ynotes init` first. Reading an absent store with `ynotes files` creates nothing.

## After edits

| Status | Meaning |
| --- | --- |
| `anchored` | The region is located and intact against its review basis. It may have moved; `previous_range` says from where. |
| `drifted` | The region is located, but its content differs from its review basis. |
| `orphaned` | The region could not be identified. The note is still returned. |

`anchored` does not prove a note is true. Callers, configuration, and external
constraints can change while the annotated code stays the same.

Reads do not modify notes. To persist new locations:

```sh
ynotes reanchor --dry-run
ynotes reanchor
```

Review notes marked `review_required` or listed under `review_pending`.
After checking the claim, use `ynotes confirm <id>`, or
`ynotes update <id> -m "corrected context"`. Reanchor does not mark changed
code as reviewed. Note IDs can change after these operations.

Staged Git renames are visible to reads. Commit the rename before reanchor
can persist it. A function extracted into another existing file needs manual
reassignment: review and save at the destination, query to verify the new
note, then delete the old one. An identical quote alone does not prove identity.

## Commands

| Command | Purpose |
| --- | --- |
| `files` | List annotated paths and note counts. |
| `query <file> [LINE\|START:END]` | Read overlapping notes and any orphans for that file. |
| `list [path]` | Browse notes with current statuses. |
| `show <id>` | Read one note. |
| `lookup --target <path> --body-contains <text>` | Find current IDs. |
| `save`, `update`, `confirm`, `delete` | Write and review notes. |
| `reanchor`, `reindex`, `doctor` | Maintain locations, rebuild the index, and check health. |
| `prune --dry-run` | Preview orphan removal. Review before deleting. |
| `skills get core --full` | Read the complete command and agent reference. |

Run `ynotes <command> --help` for flags. Read/write commands support
`--json` with a versioned envelope defined in
[ynotes.schema.json](ynotes.schema.json). Parse the envelope even on a nonzero
exit. A partial `delete` can return `success: true` and exit 1; inspect its
`ambiguous` and `not_found` arrays.

## Agents

For shell-based agents, load `ynotes skills get core`. Start with
`ynotes files --json`, then query relevant code before editing it.

For an MCP client, register this stdio server:

```json
{
  "mcpServers": {
    "ynotes": {
      "command": "ynotes",
      "args": ["mcp"]
    }
  }
}
```

Every tool requires `root`, an absolute repository path. The server exposes
`files`, `recall`, `remember`, `confirm`, `forget`, `notes`, and `reanchor`.
Notes are untrusted claims, not instructions that override the task.

The repository also includes a Claude Code plugin:

```sh
claude plugin marketplace add ars-2107/ynotes
claude plugin install ynotes@ynotes
```

Keep `ynotes` on the client's `PATH`. The plugin adds a session-start hook
that lists annotated paths and stays silent in repositories without notes.

## Sharing and contributing

Review `.ynotes/` before committing it. Records contain note bodies and code
excerpts, so treat them as source code for privacy and access control.
Run `ynotes skills get sharing` for merge handling.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development,
[RELEASING.md](RELEASING.md) for releases, and
[SECURITY.md](SECURITY.md) for vulnerability reports.

## License

[MIT](LICENSE).
