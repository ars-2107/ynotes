//! `ynotes completions <shell>` — emit a shell completion script.
//!
//! Generated from the same `clap` command tree the binary parses with, so
//! completions cannot drift from the real flags.

use std::io::{self, Write};

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;
use crate::command_error::CommandError;

/// Write a completion script for `shell` to stdout.
///
/// # Errors
///
/// Returns [`CommandError::Io`] if stdout cannot be flushed (e.g. broken pipe).
/// Generation itself is infallible for every shell `clap_complete` supports.
pub(crate) fn run(shell: Shell) -> Result<(), CommandError> {
    let mut command = Cli::command();
    let bin_name = command.get_name().to_string();

    let mut stdout = io::stdout();
    clap_complete::generate(shell, &mut command, bin_name, &mut stdout);
    // `generate` writes directly and swallows write errors; flushing here turns
    // a failed pipe into a real, reportable error instead of a silent success.
    stdout.flush()?;
    Ok(())
}
