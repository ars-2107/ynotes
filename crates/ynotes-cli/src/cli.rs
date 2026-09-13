//! The command-line surface: the argument grammar and nothing else.
//!
//! This module only *describes* the CLI with `clap`'s derive API. It performs
//! no work, parsing yields a [`Cli`] that [`crate::commands`] acts on.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// ynotes, store and retrieve code context that survives edits.
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
    /// Create or repair a `.ynotes` store in the current directory.
    Init {
        /// Emit the store path and creation status as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Save context for a file or a region of it.
    ///
    /// The location form chooses the scope: omit it for a file note,
    /// `230` for a line note, `230:327` for a range note.
    ///
    /// ynotes pays off when context will be revisited. Use it to leave
    /// breadcrumbs for an agent (or human) on a later task; a one-shot read
    /// likely does not need it. To replace a note's body in place, prefer
    /// `ynotes update <id>` over a second `save`.
    ///
    /// `--code` makes the save self-verifying: pass the region's text and the
    /// line numbers are checked against the file, corrected when the code
    /// moved, refused (with a recovery) when the quote is missing or
    /// ambiguous. Recommended whenever the location was read before edits.
    ///
    /// If no store exists yet, the first save bootstraps one at the enclosing
    /// git repository's root, so adoption needs no separate `ynotes init`. With
    /// no git repository to anchor it, the save fails and directs you to run
    /// `ynotes init` explicitly.
    Save {
        /// The file the context is about.
        file: PathBuf,

        /// `LINE` or `START:END`. Omit for a file-scoped note.
        #[arg(value_name = "LINE|START:END")]
        at: Option<String>,

        /// The region's text, exactly as the file reads now. Verifies the
        /// location: stale line numbers are corrected (the note lands where
        /// the quoted code actually is), and a quote found nowhere or at
        /// several places refuses with a recovery instead of anchoring to
        /// the wrong region. The region's length comes from `START:END` when
        /// given, else from the quote's line count.
        #[arg(long, value_name = "TEXT", requires = "at")]
        code: Option<String>,

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
    /// note for the file. Orphaned notes are always included. A record that
    /// cannot be read is surfaced (`--json` `malformed[]`), never a hard
    /// failure that hides the rest. Records omitted by the index are recovered
    /// from `notes/` without writing; JSON reports `index` health.
    ///
    /// If git shows the file was renamed from a path that has notes, those notes
    /// are surfaced here too, resolved against the current file and flagged
    /// `relocated_from` (`--json`) so an agent working at the new path finds the
    /// context without first running `reanchor`. This runs even when the file
    /// already has notes of its own (results are de-duplicated by id), so a
    /// fresh note on the renamed path never hides the pre-rename context.
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

        /// Summarise instead of printing bodies: a status breakdown (how many
        /// anchored / drifted / orphaned) so an agent can cheaply check
        /// whether there is context here before pulling it. The full,
        /// rename-aware resolution still runs; only the output is condensed.
        /// Conflicts with `--explain` (there are no per-rung details to show).
        #[arg(long, conflicts_with = "explain")]
        count: bool,
    },

    /// List every note in the store, with its current anchor status. Staged and
    /// committed rename destinations are resolved without persisting them. A
    /// record that cannot be read is surfaced (`--json` `malformed[]`), never
    /// dropped. Records omitted by the index are recovered from `notes/`
    /// without writing; JSON reports `index` health.
    List {
        /// Restrict to one file (default: every note in the store).
        file: Option<PathBuf>,

        /// Emit machine-readable JSON instead of the text format.
        #[arg(long)]
        json: bool,

        /// Also show each rung's outcome (the agreement vector) per note.
        #[arg(long)]
        explain: bool,

        /// Summarise instead of listing notes: a status breakdown across the
        /// listed set. Conflicts with `--explain`.
        #[arg(long, conflicts_with = "explain")]
        count: bool,
    },

    /// View a single note by id or hex prefix, resolved against current code.
    ///
    /// The one by-id read command: `query` selects by file and line, `list`
    /// shows the whole store, and this shows exactly one note, its body and
    /// its current anchor status (`anchored`/`drifted`/`orphaned`), resolved
    /// against the file as it is now. Identify the note by full id or any
    /// unambiguous hex prefix of at least 4 characters (`ynotes list --json`
    /// and `ynotes query --json` carry full ids); an ambiguous prefix is
    /// refused.
    ///
    /// The read is rename-aware, like `query` and `list`: a note whose file was
    /// renamed resolves against the new path and reports `relocated_to`, so all
    /// three reads agree on one note's status.
    Show {
        /// Note id, or any unambiguous hex prefix of >= 4 characters.
        id: String,

        /// Emit machine-readable JSON instead of the text format.
        #[arg(long)]
        json: bool,

        /// Also show each rung's outcome (the agreement vector).
        #[arg(long)]
        explain: bool,
    },

    /// Find note(s) by a stable handle and print their current id(s).
    ///
    /// A note's id is a content hash over its anchor and review basis, so it
    /// rotates on `reanchor`/`update`/`confirm` when those inputs change (and
    /// on a re-`save` after a commit). To reference a
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

    /// List which files carry notes, and how many each has.
    ///
    /// The orientation read, and the cheap one. `query`, `list` and `show` all
    /// resolve notes against current code, which costs the whole anchor ladder
    /// per note; this reads records and checks target paths. Missing targets
    /// use content-corroborated Git renames so discovery names the current file.
    /// Unresolved notes retain their historical path. No store is a successful
    /// empty answer (`{"store":"absent"}` under `--json`), and creates nothing.
    ///
    /// Reports no line ranges and no anchor statuses, deliberately: both would
    /// be the stored values rather than facts about the code as it is now, and
    /// a stale range dressed up as current is the quiet wrongness this tool
    /// exists to avoid. Use `query` or `list` for that.
    ///
    /// Files are listed most-annotated first, ties broken by path.
    Files {
        /// Emit machine-readable JSON instead of the text format.
        #[arg(long)]
        json: bool,
    },

    /// Refresh selectors for notes the anchor ladder can locate.
    ///
    /// A read never writes; this is the deliberate, auditable pass that
    /// persists re-anchors. Notes it cannot locate, orphaned, target
    /// missing, or a symlink-escaping target, are left untouched.
    /// An identical sibling at the saved line is not enough to locate a
    /// deleted region; without identity evidence its note stays orphaned.
    /// Re-anchoring never advances the note's review basis: when resolved code
    /// changed, the report flags `review_required` and reads stay `drifted`
    /// until `confirm` or an `update` explicitly acknowledges current code.
    ///
    /// This is also where a note durably follows a file rename: when a target
    /// file is gone but git shows it was renamed *and committed*, and a content
    /// rung confirms the region at the new path, the note is moved there and
    /// reported under `relocated[]` (`--json`). A rename where the region was
    /// deleted is not moved, it is left `orphaned` for a human, never welded
    /// onto the renamed file. A rename that is only *staged* is deferred, not
    /// migrated: the destination has no committed baseline yet, so relocating
    /// there would strip the note's git rung; `query` already surfaces the note
    /// at the new path meanwhile, and this pass migrates it once the rename is
    /// committed.
    ///
    /// Note: a re-anchored note gets a *new* id (the id is a content hash
    /// over `(target, scope, bundle, review_basis, body)`, and the bundle just changed).
    /// To track a note across reanchors, use `(target, scope, body)` as
    /// the stable lineage rather than caching the id.
    ///
    /// Re-anchoring is also the post-merge dedup pass: two copies of the same
    /// note that a concurrent merge left behind collapse into one when they
    /// re-anchor to the same place (identical bundle => identical id).
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
    /// (`ynotes list --json` and `ynotes query --json` carry full ids).
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

    /// Mark a note's context as reviewed against its current code.
    ///
    /// Re-anchoring may refresh where a note points, but it deliberately does
    /// not clear `drifted`: selector maintenance is not evidence that the note
    /// body is still true. After checking the body against the resolved code,
    /// use this command to advance its review basis and restore `anchored`.
    /// A region the ladder cannot identify is refused: an orphan, or one
    /// located only by similarity rather than by git transport, an exact
    /// quote, or its structural path. Run `ynotes reanchor` first, it
    /// refreshes the selectors, which is what makes the following confirm
    /// exact. Confirmation preserves the body but rotates the content-hash id
    /// when the review basis changed.
    Confirm {
        /// Note id, or any unambiguous hex prefix of >= 4 characters.
        id: String,

        /// Emit the confirmed note's identity as JSON (for agents/scripts).
        #[arg(long)]
        json: bool,
    },

    /// Delete one or more notes from the store, by id or hex prefix.
    ///
    /// Prefixes must be at least 4 characters and must uniquely identify a
    /// single note; an ambiguous prefix is refused (we do not silently
    /// delete multiple candidates). A prefix that matches nothing is also
    /// refused. Successful deletions still happen even when other ids in the
    /// same call fail, the process exits `1` only if anything failed.
    ///
    /// A corrupt record that cannot be read (flagged `unreadable` by `query`,
    /// `list`, and `doctor`) is removable too, but only by its *exact*
    /// 64-character id, a prefix never purges one, since there is no body to
    /// preview. Such a purge is reported separately (`deleted_unreadable` under
    /// `--json`) and is a success: it does not flip the exit code.
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
    /// An orphaned note is one no content rung could locate, its code is
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
    /// truth, the notes under `.ynotes/notes/` are. Run this if the index is
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
    /// check): version, platform, `git` availability, the discovered store with
    /// its note count, and store health, malformed note records, index drift,
    /// and missing managed `.gitattributes` merge guards. `--json` emits the
    /// same facts as a machine-readable object.
    Doctor {
        /// Emit machine-readable JSON instead of the text report.
        #[arg(long)]
        json: bool,
    },

    /// Print agent-facing usage guidance served by this binary.
    ///
    /// The content is compiled in at build time, so what it describes always
    /// matches the installed binary: `skills list` for the index,
    /// `skills get core` for the working loop, `skills get core --full` for
    /// the deep reference. Works with no store and no git repository; the
    /// output is plain markdown on stdout.
    Skills {
        /// The skills verb to run.
        #[command(subcommand)]
        action: SkillsAction,
    },

    /// Print a shell completion script to stdout.
    Completions {
        /// The shell to generate completions for (e.g. `bash`, `zsh`, `fish`).
        shell: Shell,
    },

    /// Run the MCP server over stdio.
    ///
    /// Spawned by agent clients (Claude Code, Codex, Cursor, …), not run
    /// interactively: register once with e.g.
    /// `claude mcp add ynotes -- ynotes mcp`, and the client starts and stops
    /// the process with each session. Exposes seven tools, files, recall,
    /// remember, confirm, forget, notes, reanchor, returning the same versioned payloads as
    /// `--json`.
    Mcp,

    /// Serve an agent-harness hook. Reads the harness's JSON event on stdin.
    ///
    /// Not a command to run by hand. It exists so a harness integration is one
    /// tested binary rather than a shell script with a `jq` dependency, and so
    /// the "say nothing unless there is something to say" contract is code
    /// with a test behind it instead of shell discipline.
    Hook {
        /// Which hook event is being served.
        #[command(subcommand)]
        event: HookEvent,
    },
}

/// The hook events `ynotes hook` can serve.
#[derive(Debug, Subcommand)]
pub(crate) enum HookEvent {
    /// Announce, once per session, which files in this repository carry notes.
    ///
    /// Reads the harness `SessionStart` event on stdin (for its `cwd`) and
    /// writes a short plain-text block on stdout, which the harness adds to the
    /// agent's context.
    ///
    /// This is the fix for the problem that `recall` is *speculative*: not
    /// knowing whether a file has notes, an agent skips the call, and the store
    /// goes unread no matter how good it is. Knowing the annotated files up
    /// front turns recall from discipline into an obvious, targeted action.
    ///
    /// Always exits 0, and prints nothing at all when there is no store, no
    /// note, or no readable event. It runs in every repository the agent opens,
    /// so silence everywhere else is the whole point: noise here would get the
    /// integration switched off, and the tools remain the real contract.
    SessionStart,
}

/// The verbs under `ynotes skills`.
#[derive(Debug, Subcommand)]
pub(crate) enum SkillsAction {
    /// One line per skill: name and description.
    List,

    /// Print a skill's markdown to stdout, frontmatter included.
    ///
    /// The frontmatter travels with the content so the output stays
    /// self-describing when it lands in an agent's context.
    Get {
        /// The skill to print (see `ynotes skills list`).
        name: String,

        /// Also print the deep reference appendix. Only `core` has one;
        /// requesting it on another skill is refused rather than silently
        /// serving the plain content.
        #[arg(long)]
        full: bool,
    },
}
