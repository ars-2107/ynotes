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

use std::path::PathBuf;

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
    /// file involved instead of emitting a context-free "No such file".
    #[error("i/o error at `{path}`")]
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
}
