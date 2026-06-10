//! Subcommand dispatch.
//!
//! One subcommand = one module with a single `run` entry point. [`dispatch`]
//! is the *only* place that maps a parsed [`Command`] to those entry points,
//! so the full set of behaviours is auditable from one screen.

pub(crate) mod completions;
pub(crate) mod doctor;
pub(crate) mod init;
pub(crate) mod list;
pub(crate) mod query;
pub(crate) mod reanchor;
pub(crate) mod render;
pub(crate) mod save;

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
        Command::Init => init::run(),
        Command::Save {
            file,
            at,
            message,
            json,
        } => save::run(file, at.as_deref(), message.as_deref(), *json),
        Command::Query {
            file,
            at,
            json,
            explain,
        } => query::run(file, at.as_deref(), *json, *explain),
        Command::List { file, json } => list::run(file.as_deref(), *json),
        Command::Reanchor { dry_run } => reanchor::run(*dry_run),
        Command::Doctor => doctor::run(),
        Command::Completions { shell } => completions::run(*shell),
    }
}
