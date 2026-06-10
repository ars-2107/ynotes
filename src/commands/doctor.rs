//! `ynotes doctor` — report the environment ynotes resolves against.
//!
//! Strictly read-only: it mutates nothing (invariant 8 — a resolve never
//! writes, and `doctor` writes nothing it resolves). Two jobs: make a bug
//! report self-contained (version, platform), and surface a *degraded* state
//! before it puzzles a user — chiefly a missing `git` binary, which silently
//! disables R1's transport rung.

use std::io::Write;

use crate::command_error::CommandError;

/// Write diagnostic information to stdout.
///
/// # Errors
///
/// Returns [`CommandError::Io`] if writing to stdout fails — for example when
/// the output is piped into a process that exits early (broken pipe).
pub(crate) fn run() -> Result<(), CommandError> {
    // Lock stdout once and write through the guard: fewer syscalls, and no
    // interleaving if this ever logs concurrently.
    let mut out = std::io::stdout().lock();

    writeln!(out, "{:<10}{}", "ynotes", ynotes::version())?;
    writeln!(
        out,
        "{:<10}{}-{}",
        "platform",
        std::env::consts::OS,
        std::env::consts::ARCH
    )?;

    // git is an optional runtime dependency: R1's transport rung shells out to
    // it and is skipped when absent. Report the degradation here rather than
    // leaving a user to infer it from weaker anchors.
    match ynotes::git_version() {
        Some(version) => writeln!(out, "{:<10}{version}", "git")?,
        None => writeln!(
            out,
            "{:<10}not found (R1 git transport disabled; other rungs unaffected)",
            "git"
        )?,
    }

    // The store is per-directory; doctor reports whichever one the current
    // directory resolves to, exactly as a real subcommand would. A missing
    // store is a normal state (point the user at `init`); a store that exists
    // but cannot be read is *not* — report it as the fault it is rather than
    // collapsing it into "none found".
    match ynotes::Store::discover() {
        Ok(store) => {
            let root = store.root().display();
            match store.all_notes() {
                Ok(notes) => {
                    let n = notes.len();
                    let plural = if n == 1 { "" } else { "s" };
                    writeln!(out, "{:<10}{root} ({n} note{plural})", "store")?;
                }
                Err(e) => writeln!(out, "{:<10}{root} (notes unreadable: {e})", "store")?,
            }
        }
        Err(ynotes::Error::StoreNotFound { .. }) => {
            writeln!(out, "{:<10}none found (run `ynotes init`)", "store")?;
        }
        Err(e) => writeln!(out, "{:<10}unreadable: {e}", "store")?,
    }

    Ok(())
}
