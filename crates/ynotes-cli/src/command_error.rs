//! The binary's error type and its mapping to process exit codes.
//!
//! The library returns *typed* [`ynotes::Error`]s so callers can react to the
//! kind of failure. This module does the opposite: collapse every failure into
//! something a shell understands — a message and an exit code. This is the
//! presentational half of the error strategy.

use std::process::ExitCode;

/// Process exit codes.
///
/// `2` for usage errors is the convention `clap` itself uses and matches most
/// Unix tools (`grep`, `git`); `1` is a generic runtime failure. Named so call
/// sites read as intent, not magic numbers.
pub(crate) mod codes {
    pub(crate) const FAILURE: u8 = 1;
    pub(crate) const USAGE: u8 = 2;
}

/// A presentational error for the `ynotes` binary.
///
/// Variants distinguish *how to report*, not *what went wrong* — the latter is
/// the engine's concern. `#[error(transparent)]` forwards the underlying
/// message verbatim so wrapping adds no noise.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandError {
    /// The tool was invoked incorrectly — a bad argument or value. Maps to the
    /// usage exit code (`2`).
    ///
    /// `clap` rejects argument-level usage errors before dispatch is reached;
    /// this variant carries the usage errors a subcommand can only detect once
    /// running — a malformed line range, an empty note, an `init` where a
    /// store already exists.
    #[error("{0}")]
    Usage(String),

    /// A failure that originated in the engine.
    ///
    /// Constructed via the manual [`From<ynotes::Error>`](Self::from) below
    /// rather than thiserror's `#[from]`, because a few engine errors are
    /// user-input failures dressed as engine errors (e.g.
    /// [`ynotes::Error::OutsideStore`]) and must surface as
    /// [`CommandError::Usage`] for the right exit code.
    #[error(transparent)]
    Engine(ynotes::Error),

    /// An I/O failure in the front-end itself, e.g. a closed stdout pipe.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// The `mcp` subcommand's server front-end failed to start or its stdio
    /// transport broke. A runtime failure (exit `1`); its message is forwarded
    /// verbatim. Kept distinct from the engine and the CLI's own I/O — this is
    /// the MCP front-end (`ynotes-mcp`), which owns the async runtime the CLI
    /// itself never touches. No existing variant fitted: `Usage` exits `2`,
    /// `Engine` demands a `ynotes::Error`, and `Io`/`Render` name the wrong
    /// failure class.
    #[error(transparent)]
    Mcp(#[from] ynotes_mcp::ServeError),

    /// Output could not be rendered — e.g. the `--json` agent contract failed
    /// to serialise. A runtime failure, so it maps to the failure exit code.
    #[error("cannot render output: {0}")]
    Render(String),

    /// The error has already been rendered (typically as a JSON envelope on
    /// stdout by `json_envelope::wrap`). Main only sets the exit code for
    /// this variant — it does **not** print a second diagnostic to stderr,
    /// so a `--json` invocation never mixes a JSON envelope on stdout with
    /// an unstructured line on stderr.
    #[error("error already rendered (exit {code})")]
    Rendered {
        /// The raw exit code to surface — preserved from the underlying
        /// error so usage failures (`2`) and runtime failures (`1`) stay
        /// distinguishable to the shell.
        code: u8,
    },
}

/// Classifies an engine error: most fall through to [`CommandError::Engine`]
/// (exit `1`), but the user-input failures the engine exposes —
/// [`ynotes::Error::OutsideStore`], [`ynotes::Error::InvalidLocation`],
/// [`ynotes::Error::InvalidIdPrefix`], and [`ynotes::Error::SymlinkEscape`] —
/// surface as [`CommandError::Usage`] (exit `2`). This is the single place
/// that decision is made, so a fresh engine call only needs `?` to get the
/// right classification.
impl From<ynotes::Error> for CommandError {
    fn from(e: ynotes::Error) -> Self {
        match e {
            ynotes::Error::OutsideStore { .. }
            | ynotes::Error::InvalidLocation(_)
            | ynotes::Error::InvalidIdPrefix(_)
            | ynotes::Error::SymlinkEscape { .. } => CommandError::Usage(e.to_string()),
            other => CommandError::Engine(other),
        }
    }
}

impl CommandError {
    /// The process exit code this error should produce, as the raw byte.
    pub(crate) fn exit_code_byte(&self) -> u8 {
        match self {
            CommandError::Usage(_) => codes::USAGE,
            CommandError::Rendered { code } => *code,
            CommandError::Engine(_)
            | CommandError::Io(_)
            | CommandError::Render(_)
            | CommandError::Mcp(_) => codes::FAILURE,
        }
    }

    /// The process exit code this error should produce.
    ///
    /// Usage errors return `2`; every other failure returns `1`. A
    /// [`Rendered`](CommandError::Rendered) carries its own code so the
    /// JSON-envelope wrapper can preserve the underlying classification.
    pub(crate) fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.exit_code_byte())
    }
}
