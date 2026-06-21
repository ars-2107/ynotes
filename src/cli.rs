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
    ///
    /// ynotes pays off when context will be revisited. Use it to leave
    /// breadcrumbs for an agent (or human) on a later task; a one-shot read
    /// likely does not need it. To replace a note's body in place, prefer
    /// `ynotes update <id>` over a second `save`.
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

    /// Find note(s) by a stable handle and print their current id(s).
    ///
    /// A note's id is a content hash over its anchor, so it rotates on every
    /// `reanchor`/`update` (and on a re-`save` after a commit). To reference a
    /// note durably (a PR description, a code comment), record the
    /// `(target, body)` it is about and resolve it back to the live id with
    /// this command. At least one of `--target`/`--body-contains` is required.
    Lookup {
        /// Restrict to a file or directory (store-relative, like `list`).
        #[arg(long, value_name = "PATH")]
        target: Option<PathBuf>,

        /// Keep only notes whose body contains this substring (case-sensitive).
        #[arg(long, value_name = "TEXT")]
        body_contains: Option<String>,

        /// Emit machine-readable JSON instead of the text format.
        #[arg(long)]
        json: bool,
    },

    /// Refresh selectors for high-confidence notes.
    ///
    /// A read never writes; this is the deliberate, auditable pass that
    /// persists re-anchors. Low-confidence and orphaned notes are left
    /// untouched.
    ///
    /// Note: a re-anchored note gets a *new* id (the id is a content hash
    /// over `(target, scope, bundle, body)`, and the bundle just changed).
    /// To track a note across reanchors, use `(target, scope, body)` as
    /// the stable lineage rather than caching the id.
    Reanchor {
        /// Show what would change without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Emit machine-readable JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },

    /// Replace an existing note's body in place.
    ///
    /// The note's target file, scope, and line range are reused; the
    /// selector bundle is re-captured against the current file. The new note
    /// supersedes the old, retiring its content-hash id. Identify the note
    /// by full id or any unambiguous hex prefix of at least 4 characters
    /// (`yn list --json` and `yn query --json` carry full ids).
    Update {
        /// Note id, or any unambiguous hex prefix of >= 4 characters.
        id: String,

        /// The new context text. If omitted, it is read from stdin.
        #[arg(short, long)]
        message: Option<String>,

        /// Emit the updated note's identity as JSON (for agents/scripts).
        #[arg(long)]
        json: bool,
    },

    /// Delete one or more notes from the store, by id or hex prefix.
    ///
    /// Prefixes must be at least 4 characters and must uniquely identify a
    /// single note; an ambiguous prefix is refused (we do not silently
    /// delete multiple candidates). A prefix that matches nothing is also
    /// refused. Successful deletions still happen even when other ids in the
    /// same call fail — the process exits `1` only if anything failed.
    Delete {
        /// Note ids (or unambiguous hex prefixes, >= 4 characters).
        #[arg(required = true, value_name = "ID")]
        ids: Vec<String>,

        /// Show what would be deleted without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Emit machine-readable JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },

    /// Remove every orphaned note from the store.
    ///
    /// An orphaned note is one no content rung could locate — its code is
    /// gone or wholly rewritten. ynotes never auto-prunes (an orphan may be
    /// historically valuable, or its target may be temporarily missing), so
    /// this is the deliberate cleanup pass.
    Prune {
        /// Show what would be removed without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Emit machine-readable JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },

    /// Rebuild the by-path index from the notes on disk.
    ///
    /// The index (`.ynotes/index/by-path.json`) is a cache, not the source of
    /// truth — the notes under `.ynotes/notes/` are. Run this if the index is
    /// lost, hand-edited, or out of step with the notes (a query or delete
    /// that logs "run a reindex" is the signal). It recovers any note on disk
    /// the index forgot and drops any pointer with no note behind it; it never
    /// touches the notes themselves, so it cannot lose context.
    Reindex {
        /// Show what would be rebuilt without writing anything.
        #[arg(long)]
        dry_run: bool,

        /// Emit machine-readable JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },

    /// Report the environment ynotes resolves against (read-only health
    /// check): version, platform, `git` availability, and the discovered
    /// store with its note count. `--json` emits the same facts as a
    /// machine-readable object.
    Doctor {
        /// Emit machine-readable JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },

    /// Print a shell completion script to stdout.
    Completions {
        /// The shell to generate completions for (e.g. `bash`, `zsh`, `fish`).
        shell: Shell,
    },
}
