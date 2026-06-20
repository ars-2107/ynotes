//! Subcommand dispatch.
//!
//! One subcommand = one module with a single `run` entry point. [`dispatch`]
//! is the *only* place that maps a parsed [`Command`] to those entry points,
//! so the full set of behaviours is auditable from one screen.

pub(crate) mod completions;
pub(crate) mod delete;
pub(crate) mod doctor;
pub(crate) mod id;
pub(crate) mod init;
pub(crate) mod json_envelope;
pub(crate) mod list;
pub(crate) mod prune;
pub(crate) mod query;
pub(crate) mod reanchor;
pub(crate) mod reindex;
pub(crate) mod render;
pub(crate) mod safety;
pub(crate) mod save;
pub(crate) mod update;

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
        Command::Reanchor { dry_run, json } => reanchor::run(*dry_run, *json),
        Command::Update { id, message, json } => update::run(id, message.as_deref(), *json),
        Command::Delete { ids, dry_run, json } => delete::run(ids, *dry_run, *json),
        Command::Prune { dry_run, json } => prune::run(*dry_run, *json),
        Command::Reindex { dry_run, json } => reindex::run(*dry_run, *json),
        Command::Doctor { json } => doctor::run(*json),
        Command::Completions { shell } => completions::run(*shell),
    }
}
