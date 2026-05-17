//! The binary's error type and its mapping to process exit codes.
//!
//! The library returns *typed* [`ynotes::Error`]s so callers can react to the
//! kind of failure. This module does the opposite: collapse every failure into
//! something a shell understands — a message and an exit code. This is the
//! presentational half of the error strategy (jujutsu's `command_error.rs`).

use std::process::ExitCode;

/// Process exit codes.
///
/// `2` for usage errors is the convention `clap` itself uses and matches most
/// Unix tools (`grep`, `git`); `1` is a generic runtime failure. Named so call
/// sites read as intent, not magic numbers.
mod codes {
    pub(super) const FAILURE: u8 = 1;
    pub(super) const USAGE: u8 = 2;
}

/// A presentational error for the `ynotes` binary.
///
/// Variants distinguish *how to report*, not *what went wrong* — the latter is
/// the engine's concern. `#[error(transparent)]` forwards the underlying
/// message verbatim so wrapping adds no noise.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandError {
    /// The tool was invoked incorrectly (bad argument or value). Maps to the
    /// usage exit code.
    ///
    /// Not yet constructed: `clap` handles argument-level usage errors itself
    /// (exiting `2`) before dispatch is reached. This variant is for
    /// subcommands that validate their *own* input (e.g. a malformed line
    /// range) and is part of the stable exit-code contract, so it is kept
    /// ahead of its first caller rather than added later.
    #[allow(dead_code)]
    #[error("{0}")]
    Usage(String),

    /// A failure that originated in the engine.
    #[error(transparent)]
    Engine(#[from] ynotes::Error),

    /// An I/O failure in the front-end itself, e.g. a closed stdout pipe.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl CommandError {
    /// The process exit code this error should produce.
    ///
    /// Usage errors return `2`; engine and I/O failures return `1`.
    pub(crate) fn exit_code(&self) -> ExitCode {
        let code = match self {
            CommandError::Usage(_) => codes::USAGE,
            CommandError::Engine(_) | CommandError::Io(_) => codes::FAILURE,
        };
        ExitCode::from(code)
    }
}
