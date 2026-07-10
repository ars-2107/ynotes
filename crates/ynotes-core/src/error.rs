//! The error type returned by every fallible operation in the engine.
//!
//! The engine returns a single concrete [`Error`] enum (built with
//! `thiserror`), not a boxed or dynamic error. The reason is the caller: the
//! binary maps failures to process exit codes and user-facing messages, so it
//! must be able to *match on the kind* of failure. A `Box<dyn Error>` would
//! erase exactly the information the front-end exists to act on.
//!
//! This is the library half of the error strategy in AGENTS.md: typed,
//! matchable errors here; presentational errors (with exit codes) in the
//! binary's `command_error` module.

use std::path::{Path, PathBuf};

/// Render an `Error::Io` for `Display`. Lifted out of the `#[error(...)]`
/// attribute so the `NotFound` case can take a different wording from a
/// generic I/O failure without conditionalising the format string itself.
fn display_io_error(path: &Path, source: &std::io::Error) -> String {
    match source.kind() {
        std::io::ErrorKind::NotFound => {
            format!("`{}` does not exist (check the path)", path.display())
        }
        _ => format!("i/o error at `{}`: {source}", path.display()),
    }
}

/// Render an `Error::CodeAmbiguous` for `Display`. Lifted out of the
/// `#[error(...)]` attribute because joining the candidate lines needs real
/// code. The wording is agent-facing contract, not decoration: the MCP
/// surface flattens the variant to this string, so the message is the whole
/// recovery protocol at the point of failure.
fn display_code_ambiguous(lines: &[u32]) -> String {
    // Cap the roll-call: a one-line quote in a big file can match hundreds of
    // places, and past a point the list stops aiding recovery and starts
    // burying the instruction after it. The typed variant keeps every
    // candidate; only the rendering is capped.
    const LISTED: usize = 10;
    let joined = lines
        .iter()
        .take(LISTED)
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let more = lines.len().saturating_sub(LISTED);
    let listed = if more > 0 {
        format!("{joined}, and {more} more")
    } else {
        joined
    };
    format!(
        "the quoted code occurs {} times (lines {listed}) and start matches none of them; \
         correct start, or quote more surrounding lines",
        lines.len()
    )
}

/// A specialised [`Result`](std::result::Result) for engine operations.
///
/// Defaulting the error parameter to [`Error`] lets call sites write
/// `Result<T>` while still allowing `Result<T, OtherError>` where useful.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong inside the ynotes engine.
///
/// Variants are deliberately coarse. They exist so a front-end can decide how
/// to *present* a failure, not to enumerate every possible cause. Add a variant
/// when a caller must react differently to it — not merely to attach a new
/// error string.
///
/// `#[non_exhaustive]`: downstream `match`es must include a wildcard arm, so
/// new variants can be added without a breaking change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An I/O operation failed.
    ///
    /// The path is carried alongside the OS error so messages can name the
    /// file involved instead of emitting a context-free "No such file". The
    /// `Display` form special-cases `NotFound` to match the wording the
    /// query path already uses for a missing target, so the same condition
    /// reads the same way wherever the front-end surfaces it.
    #[error("{}", display_io_error(path, source))]
    Io {
        /// The filesystem path the failed operation targeted.
        path: PathBuf,
        /// The underlying operating-system error.
        #[source]
        source: std::io::Error,
    },

    /// The on-disk store or configuration was readable but malformed.
    ///
    /// The string is a human-readable explanation suitable for showing to the
    /// user; it should describe what was expected, not just what was found.
    #[error("invalid ynotes data: {0}")]
    Invalid(String),

    /// No store was found at or above the search origin.
    ///
    /// Distinct from [`Error::Invalid`] because the front-end's reaction
    /// differs: this warrants pointing the user at `ynotes init`, not
    /// reporting corruption.
    #[error("no ynotes store found from `{searched_from}` upward (run `ynotes init`)")]
    StoreNotFound {
        /// The directory the upward search began from.
        searched_from: PathBuf,
    },

    /// A user-supplied path resolves outside the store's work tree.
    ///
    /// Distinct from [`Error::Invalid`] because the front-end maps this to a
    /// *usage* exit code (`2`) — the user pointed at the wrong place, not a
    /// runtime failure. Raised by [`crate::Store::relativize`] when the
    /// absolute form of the given path does not sit under the store root.
    #[error("`{}` is outside the ynotes store at `{}`", path.display(), store_root.display())]
    OutsideStore {
        /// The path the caller supplied.
        path: PathBuf,
        /// The work tree the path was expected to sit beneath.
        store_root: PathBuf,
    },

    /// A user-supplied location (line/range) is invalid for the target file —
    /// line 0, an inverted range, or a range past EOF. Front-ends map this to
    /// their usage class (the CLI exits `2`): the user pointed at the wrong
    /// place, nothing failed at runtime.
    #[error("{0}")]
    InvalidLocation(String),

    /// A user-supplied note-id prefix is malformed — shorter than the minimum
    /// [`crate::validate_id_prefix`] accepts, or not hexadecimal. Front-ends
    /// map this to their usage class (the CLI exits `2`): the caller typed a
    /// selector that cannot name any note, which is a call-site mistake, not a
    /// runtime failure. Kept distinct from [`Error::InvalidLocation`] (the
    /// line/range family) so a front-end can react to each independently.
    #[error("{0}")]
    InvalidIdPrefix(String),

    /// The `code` quote passed to a verified save was found nowhere in the
    /// target file: the caller holds a stale read of the file, or the wrong
    /// file entirely. Front-ends map this to their usage class (the CLI exits
    /// `2`). The `Display` text carries the recovery — re-read and re-quote —
    /// because the MCP surface flattens the variant into its message.
    #[error(
        "the quoted code is not in this file; re-read the file and quote the region as it reads now"
    )]
    CodeNotFound,

    /// The `code` quote passed to a verified save occurs at more than one
    /// place in the target file and none of them is the declared start: the
    /// coordinates are stale *and* the quote collides. Carries every
    /// candidate so the retry is one step — correct `start` to one of them,
    /// or quote more lines. Usage class, like [`Error::CodeNotFound`]; the
    /// `Display` text is the recovery protocol.
    #[error("{}", display_code_ambiguous(lines))]
    CodeAmbiguous {
        /// Every 1-based line where the quote matches, ascending.
        lines: Vec<u32>,
    },

    /// A *symlink* whose canonical target lives outside the store's work tree.
    /// Kept distinct from [`Error::OutsideStore`] so the message can name the
    /// symlink resolution; both are usage-class for a front-end.
    #[error("`{}` is a symlink whose target escapes the work tree at `{}` (resolves to `{}`)",
        path.display(), workdir.display(), resolved.display())]
    SymlinkEscape {
        /// The symlink the caller supplied.
        path: PathBuf,
        /// The work tree the target was expected to sit beneath.
        workdir: PathBuf,
        /// Where the symlink actually resolves.
        resolved: PathBuf,
    },

    /// The by-path index was readable but parseable in neither the current
    /// (JSONL) nor the legacy (single-object) format — typically a merge that
    /// mangled the cache. The index is rebuildable, so the remedy is carried in
    /// the message (mirrors [`Error::StoreNotFound`]'s embedded hint).
    #[error("index `{}` is malformed; run `ynotes reindex` to rebuild it from your notes", path.display())]
    IndexMalformed {
        /// The index file that failed to parse.
        path: PathBuf,
    },

    /// The by-path index file is absent while `notes/` holds records — the cache
    /// was deleted or a merge removed it, but the notes survive. Distinct from
    /// "empty store" (no notes), which is not an error. The index is
    /// rebuildable, so the remedy rides in the message, like [`Error::IndexMalformed`].
    #[error("index `{}` is missing; run `ynotes reindex` to rebuild it from your notes", path.display())]
    IndexMissing {
        /// The index file that was expected but absent.
        path: PathBuf,
    },
}
