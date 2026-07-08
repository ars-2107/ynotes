//! The server `instructions` — the loop protocol every MCP client injects at
//! session start. Static by design: dynamic content here (timestamps, paths)
//! would bust client prompt caches. ≤2KB (Claude Code truncates beyond that).

/// Injected into the agent's context by the MCP initialize handshake.
pub(crate) const INSTRUCTIONS: &str = "\
ynotes stores context notes anchored to code regions (file + line range), \
committed to the repo and shared with the team. Notes re-anchor as code \
evolves and report honest status: anchored (trust it), drifted (region moved \
or changed - verify the note against the current code), orphaned (region \
gone - historical context only).

PROTOCOL:
1. Before editing or reviewing a file you have not read this session, call \
recall on it - count: true first if you only need to know whether context \
exists. Code that looks wrong may have a note explaining why it is deliberate.
2. When you finish non-trivial work, record what would surprise a competent \
reader in three months: constraints, invariants, gotchas, cross-file \
couplings, rejected approaches and why. Call remember with the smallest \
region the context is about. Do NOT record what the code plainly shows - a \
low-signal store is worse than none.
3. If a note is wrong or obsolete, forget it. To correct one, remember at \
its current region; if the old note still shows in recall afterwards, \
forget it.
4. After work that moved or heavily edited annotated code, run reanchor \
(dry_run first if unsure).

Write notes standalone, for a reader without this session's context.";
