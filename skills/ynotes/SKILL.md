---
name: ynotes
description: Code-anchored context notes. Use when editing annotated files, recalling or saving code context, or recording a non-obvious constraint. Load ynotes skills get core before first use.
allowed-tools: Bash(ynotes:*)
user-invocable: false
---

# ynotes

The CLI must be on `PATH` as `ynotes`. Check with `ynotes doctor`.
Build from a checkout with `cargo build --release --locked --bin ynotes` if
a released binary is not available.

Load the usage guide shipped with the installed binary:

```sh
ynotes skills get core
```

Additional guides:

- `ynotes skills get sharing` for Git stores and merge handling.
- `ynotes skills get maintenance` for drift, orphans, and index repair.
- `ynotes skills get core --full` for command flags and payloads.

Prefer MCP tools when the server is registered. Every call requires `root`,
the absolute repository path. Relative file arguments resolve beneath it;
the server never infers the repository from its process directory.
