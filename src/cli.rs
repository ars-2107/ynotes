//! The command-line surface: the argument grammar and nothing else.
//!
//! This module only *describes* the CLI with `clap`'s derive API. It performs
//! no work — parsing yields a [`Cli`] that [`crate::commands`] acts on.

use std::path::PathBuf;

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
    /// Create a `.ynotes` store in the current directory.
    Init,

    /// Save context for a file or a region of it.
    ///
    /// The location form chooses the scope: omit it for a file note,
    /// `230` for a line note, `230:327` for a range note.
    Save {
        /// The file the context is about.
        file: PathBuf,

        /// `LINE` or `START:END`. Omit for a file-scoped note.
        #[arg(value_name = "LINE|START:END")]
        at: Option<String>,

        /// The context text. If omitted, it is read from stdin.
        #[arg(short, long)]
        message: Option<String>,

        /// Emit the saved note's identity as JSON (for agents/scripts).
        #[arg(long)]
        json: bool,
    },

    /// Return the context overlapping a file or region.
    ///
    /// `230` returns that line's notes and any range/file note covering it;
    /// `230:327` returns notes overlapping the range; with no location, every
    /// note for the file. Orphaned notes are always included.
    Query {
        /// The file to query.
        file: PathBuf,

        /// `LINE` or `START:END`. Omit to return every note for the file.
        #[arg(value_name = "LINE|START:END")]
        at: Option<String>,

        /// Emit machine-readable JSON instead of the text format.
        #[arg(long)]
        json: bool,

        /// Also show each rung's outcome (the agreement vector).
        #[arg(long)]
        explain: bool,
    },

    /// List every note in the store, with its current anchor status.
    List {
        /// Restrict to one file (default: every note in the store).
        file: Option<PathBuf>,

        /// Emit machine-readable JSON instead of the text format.
        #[arg(long)]
        json: bool,
    },

    /// Refresh selectors for high-confidence notes (ynotes's `git gc`).
    ///
    /// A read never writes; this is the deliberate, auditable pass that
    /// persists re-anchors. Low-confidence and orphaned notes are left
    /// untouched.
    Reanchor {
        /// Show what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Report the environment ynotes resolves against (read-only health
    /// check): version, platform, `git` availability, and the discovered
    /// store with its note count.
    Doctor,

    /// Print a shell completion script to stdout.
    Completions {
        /// The shell to generate completions for (e.g. `bash`, `zsh`, `fish`).
        shell: Shell,
    },
}
