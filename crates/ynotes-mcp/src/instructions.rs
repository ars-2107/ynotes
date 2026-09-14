//! The server `instructions`, the loop protocol every MCP client injects at
//! session start. Static by design: dynamic content here (timestamps, paths)
//! would bust client prompt caches. ≤2KB (Claude Code truncates beyond that;
//! enforced by the test below).

/// Injected into the agent's context by the MCP initialize handshake.
pub(crate) const INSTRUCTIONS: &str = "\
ynotes stores notes anchored to code regions (file + line range), \
committed to the repo. Notes re-anchor as code \
evolves and report honest status: anchored (region intact, possibly moved; not proof the claim is true), \
drifted (region content changed - verify the note against the current code), orphaned (region \
gone - historical context only).

PROTOCOL:
Every tool call requires root: the repository's absolute path. Reuse the same \
root for a task; never infer it from the server process.
1. Call files once per repository: it cheaply names every annotated file, so \
recall stops being a guess. Before editing or reviewing one of those files, \
call recall on it. Code that looks wrong may have a note explaining why it is \
deliberate.
2. When you finish non-trivial work, record what would surprise a competent \
reader in three months: constraints, invariants, gotchas, cross-file \
couplings, rejected approaches and why. Do NOT record what the code plainly \
shows - a low-signal store is worse than none.
3. Call remember with the smallest region the context is about, passing code \
(the region's text exactly as the file reads now) alongside start. Line \
numbers you read earlier go stale as you edit; the quote corrects them, and \
saveData.range reports where the note landed. Indentation \
is ignored, the text is not. For a long region, quote its first lines and \
pass end. On not-found, re-read the file and quote what is actually there; \
on ambiguous, correct start to a listed candidate or quote more lines. Never \
drop code to make an error go away.
4. If a note is wrong or obsolete, forget it. To correct one, remember at \
its current region; forget the old one if recall still shows it.
5. After work that moved or edited annotated code, run reanchor (dry_run \
first if unsure). Reanchor never clears stale context. For each \
review_required note AND each review_pending entry, verify its body against \
current code, then confirm if still true or remember a corrected note if not.

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
