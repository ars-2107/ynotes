//! Subcommand dispatch.
//!
//! One subcommand = one module with a single `run` entry point. [`dispatch`]
//! is the *only* place that maps a parsed [`Command`] to those entry points,
//! so the full set of behaviours is auditable from one screen.

pub(crate) mod completions;
pub(crate) mod doctor;

use crate::cli::{Cli, Command};
use crate::command_error::CommandError;

/// Route a parsed invocation to the matching subcommand.
///
/// # Errors
///
/// Propagates the dispatched subcommand's error unchanged; see each
/// subcommand's `run` for its specific failure modes.
pub(crate) fn dispatch(cli: &Cli) -> Result<(), CommandError> {
    match &cli.command {
        Command::Doctor => doctor::run(),
        Command::Completions { shell } => completions::run(*shell),
    }
}
