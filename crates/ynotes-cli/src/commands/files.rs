//! Render annotated paths and counts from [`ynotes::files`].
//! The core handles missing targets and corroborated pending renames.

use std::io::Write as _;

use ynotes::Store;
use ynotes::contract::FilesData;

use super::json_envelope;
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// List the annotated files.
///
/// # Errors
///
/// [`CommandError::Engine`] if the store cannot be discovered or its records
/// cannot be enumerated. Under `--json`, any failure is rendered as the
/// agent-contract failure envelope on stdout and returned as
/// [`CommandError::Rendered`].
pub(crate) fn run(json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(run_json)
    } else {
        run_text()
    }
}

fn run_json() -> Result<(), CommandError> {
    let store = match Store::discover() {
        Ok(store) => store,
        Err(ynotes::Error::StoreNotFound { .. }) => {
            return json_envelope::print_success(&serde_json::json!({"store": "absent"}));
        }
        Err(error) => return Err(CommandError::Engine(error)),
    };
    let inventory = ynotes::files(&store)?;
    json_envelope::print_success(&FilesData::new(&inventory))
}

fn run_text() -> Result<(), CommandError> {
    let store = match Store::discover() {
        Ok(store) => store,
        Err(ynotes::Error::StoreNotFound { .. }) => {
            writeln!(
                std::io::stdout().lock(),
                "No ynotes store. Run `ynotes init` to start recording context."
            )?;
            return Ok(());
        }
        Err(error) => return Err(CommandError::Engine(error)),
    };
    let inventory = ynotes::files(&store)?;

    let mut out = std::io::stdout().lock();
    for file in &inventory.files {
        // Count first: the column an eye scans when deciding where the context
        // actually is.
        writeln!(
            out,
            "{:>4}  {}",
            paint(&file.notes.to_string(), Colour::Dim),
            file.target,
        )?;
    }
    let malformed = if inventory.malformed > 0 {
        format!(", {} malformed", inventory.malformed)
    } else {
        String::new()
    };
    writeln!(
        out,
        "{}",
        paint(
            &format!(
                "{} note(s) across {} file(s){malformed}",
                inventory.total,
                inventory.files.len(),
            ),
            Colour::Dim,
        )
    )?;
    Ok(())
}
