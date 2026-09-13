//! Ensure a valid store exists in the current directory.
//! Repeated initialisation succeeds without replacing existing records.

use std::io::Write as _;

use ynotes::contract::InitData;

use super::json_envelope;
use crate::command_error::CommandError;

/// Ensure the store exists, then report what happened.
///
/// # Errors
///
/// Returns [`CommandError::Io`] if the current directory cannot be
/// determined, or [`CommandError::Engine`] if `.ynotes` exists but is not a
/// valid store, or the store cannot be written.
pub(crate) fn run(json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(run_json)
    } else {
        run_text()
    }
}

fn initialise() -> Result<(ynotes::Store, bool), CommandError> {
    let cwd = std::env::current_dir()?;
    // Detect existence before `init` so the report distinguishes "created"
    // from "already present" without complicating the library's return type.
    let was_existing = cwd.join(".ynotes").join("HEAD").is_file();
    let store = ynotes::Store::init(&cwd).map_err(CommandError::Engine)?;
    Ok((store, !was_existing))
}

fn run_json() -> Result<(), CommandError> {
    let (store, created) = initialise()?;
    json_envelope::print_success(&InitData {
        root: store.root().to_string_lossy().into_owned(),
        created,
    })
}

fn run_text() -> Result<(), CommandError> {
    let (store, created) = initialise()?;
    let mut out = std::io::stdout().lock();
    if created {
        writeln!(
            out,
            "initialised empty ynotes store at {}",
            store.root().display()
        )?;
    } else {
        writeln!(
            out,
            "ynotes store already exists at {}",
            store.root().display()
        )?;
    }
    Ok(())
}
