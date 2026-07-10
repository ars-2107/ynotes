//! The server `instructions` — the loop protocol every MCP client injects at
//! session start. Static by design: dynamic content here (timestamps, paths)
//! would bust client prompt caches. ≤2KB (Claude Code truncates beyond that;
//! enforced by the test below).

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
couplings, rejected approaches and why. Do NOT record what the code plainly \
shows - a low-signal store is worse than none.
3. Call remember with the smallest region the context is about, passing code \
(the region's text exactly as the file reads now) alongside start. Line \
numbers you read earlier go stale as you edit; the quote verifies and \
corrects them, and saveData.range reports where the note landed. Indentation \
is ignored, the text is not. For a long region, quote its first lines and \
pass end. On not-found, re-read the file and quote what is actually there; \
on ambiguous, correct start to a listed candidate or quote more lines. Never \
drop code to make an error go away.
4. If a note is wrong or obsolete, forget it. To correct one, remember at \
its current region; if the old note still shows in recall afterwards, \
forget it.
5. After work that moved or heavily edited annotated code, run reanchor \
(dry_run first if unsure).

Write notes standalone, for a reader without this session's context.";

#[cfg(test)]
mod tests {
    /// Claude Code truncates server instructions beyond 2KB. An overrun would
    /// silently cut the protocol's tail in every session with nothing
    /// failing, so the cap is a test, not a comment.
    #[test]
    fn instructions_fit_the_2kb_client_cap() {
        let len = super::INSTRUCTIONS.len();
        assert!(len <= 2048, "instructions are {len} bytes, cap is 2048");
    }
}
