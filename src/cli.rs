//! The command-line surface: the argument grammar and nothing else.
//!
//! This module only *describes* the CLI with `clap`'s derive API. It performs
//! no work — parsing yields a [`Cli`] that [`crate::commands`] acts on.

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// ynotes — store and retrieve code context that survives edits.
#[derive(Debug, Parser)]
#[command(name = "ynotes", version, about, long_about = None, propagate_version = true)]
pub(crate) struct Cli {
    /// Increase logging verbosity. Repeat for more: `-v` info, `-vv` debug,
    /// `-vvv` trace. Overridden by the `YNOTES_LOG` env var.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub(crate) verbose: u8,

    /// The subcommand to run.
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Every ynotes subcommand.
///
/// Each variant maps to exactly one module in [`crate::commands`]. Adding a
/// subcommand is: a variant here, a module there, one arm in
/// `commands::dispatch`.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Report the environment ynotes sees (read-only health check).
    Doctor,

    /// Print a shell completion script to stdout.
    Completions {
        /// The shell to generate completions for (e.g. `bash`, `zsh`, `fish`).
        shell: Shell,
    },
}
