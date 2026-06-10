//! `ynotes init` — ensure a `.ynotes` store exists in the current directory.
//!
//! Idempotent: a pre-existing valid store is acknowledged with exit `0`, not
//! refused. The user asked for "a store is present here", which is already
//! true.

use std::io::Write as _;

use crate::command_error::CommandError;

/// Ensure the store exists, then report what happened.
///
/// # Errors
///
/// Returns [`CommandError::Io`] if the current directory cannot be
/// determined, or [`CommandError::Engine`] if `.ynotes` exists but is not a
/// valid store, or the store cannot be written.
pub(crate) fn run() -> Result<(), CommandError> {
    let cwd = std::env::current_dir()?;
    // Detect existence before `init` so the report distinguishes "created"
    // from "already present" without complicating the library's return type.
    let was_existing = cwd.join(".ynotes").join("HEAD").is_file();
    let store = ynotes::Store::init(&cwd).map_err(CommandError::Engine)?;
    let mut out = std::io::stdout().lock();
    if was_existing {
        writeln!(
            out,
            "ynotes store already exists at {}",
            store.root().display()
        )?;
    } else {
        writeln!(
            out,
            "initialised empty ynotes store at {}",
            store.root().display()
        )?;
    }
    Ok(())
}
