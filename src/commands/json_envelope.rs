//! Shared envelope for every `--json` output.
//!
//! Shape (`v: 8` of the agent contract):
//!
//! - success: `{"success": true, "v": 8, "data": <command payload>}`
//! - failure: `{"success": false, "v": 8, "error": "<message>", "type": "<kind>"}`
//!
//! Both shapes are emitted to **stdout**, including the failure case — a
//! JSON-typed consumer never has to branch on the exit code to parse the
//! response, and the `success` field is the in-band signal.
//!
//! The diagnostic still leaves stderr untouched for that path (main suppresses
//! its usual `ynotes: <err>` line when an error has already been rendered as
//! JSON), so a `--json` invocation produces exactly one stream of output.

use std::io::Write as _;

use serde::Serialize;

use crate::command_error::CommandError;

/// The current `--json` contract version. Bumped when any field changes;
/// mirrored in `tests/agent_contract.rs` and `ynotes.schema.json`
/// (invariant #7).
pub(crate) const CONTRACT_VERSION: u8 = 8;

/// The success envelope: `{success: true, v: N, data: <payload>}`. Declared
/// as a struct (not built with `json!`) so serde preserves the field order
/// `success → v → data`, keeping the contract snapshot deterministic.
#[derive(Serialize)]
struct SuccessEnvelope<T: Serialize> {
    success: bool,
    v: u8,
    data: T,
}

/// The failure envelope: `{success: false, v: N, error: …, type: …}`.
#[derive(Serialize)]
struct FailureEnvelope<'a> {
    success: bool,
    v: u8,
    error: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
}

/// Emit a success envelope wrapping `data` and write it to stdout.
///
/// # Errors
///
/// [`CommandError::Render`] if `data` cannot be serialised, or
/// [`CommandError::Io`] if the write to stdout fails.
pub(crate) fn print_success<T: Serialize>(data: &T) -> Result<(), CommandError> {
    let envelope = SuccessEnvelope {
        success: true,
        v: CONTRACT_VERSION,
        data,
    };
    let line =
        serde_json::to_string_pretty(&envelope).map_err(|e| CommandError::Render(e.to_string()))?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}")?;
    Ok(())
}

/// Emit a failure envelope to stdout. Best-effort: a serialisation failure
/// here falls back to a hand-built string so the consumer still sees parseable
/// JSON.
pub(crate) fn print_error(message: &str, kind: ErrorKind) {
    let envelope = FailureEnvelope {
        success: false,
        v: CONTRACT_VERSION,
        error: message,
        kind: kind.as_str(),
    };
    let line = serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| {
        r#"{"success":false,"v":8,"error":"failed to render JSON error envelope","type":"render"}"#
            .to_string()
    });
    // Best-effort write — a broken pipe here is unrecoverable and the exit
    // code will still be set by the caller.
    let _written = writeln!(std::io::stdout().lock(), "{line}");
}

/// Stable enum surfaced in the `type` field of a failure envelope so a
/// consumer can branch programmatically instead of regex-matching the
/// message.
#[derive(Copy, Clone, Debug)]
pub(crate) enum ErrorKind {
    /// Bad invocation or bad user-supplied value (exit `2`).
    Usage,
    /// An engine-level failure (exit `1`).
    Engine,
    /// An I/O failure in the binary itself (exit `1`).
    Io,
    /// Output could not be serialised (exit `1`).
    Render,
}

impl ErrorKind {
    fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Usage => "usage",
            ErrorKind::Engine => "engine",
            ErrorKind::Io => "io",
            ErrorKind::Render => "render",
        }
    }
}

impl From<&CommandError> for ErrorKind {
    fn from(e: &CommandError) -> Self {
        // `Rendered` shares the `Render` class as a defensive default — by
        // construction it is only produced inside [`wrap`] and should not
        // reach this conversion, so treating it as render-class is the
        // conservative wildcard.
        match e {
            CommandError::Usage(_) => ErrorKind::Usage,
            CommandError::Engine(_) => ErrorKind::Engine,
            CommandError::Io(_) => ErrorKind::Io,
            CommandError::Render(_) | CommandError::Rendered { .. } => ErrorKind::Render,
        }
    }
}

/// Wrap a fallible `--json` command body: on `Err`, render the failure
/// envelope to stdout and convert the error into the rendered sentinel so
/// `main` does not also print a stderr line.
///
/// A body that returns [`CommandError::Rendered`] is passed through
/// unchanged — it already emitted its own envelope (typically a *success*
/// envelope alongside a per-call non-zero exit code, as `delete` does when
/// some requested ids were ambiguous or missing). Otherwise stdout would
/// carry two JSON documents back-to-back.
pub(crate) fn wrap<F>(body: F) -> Result<(), CommandError>
where
    F: FnOnce() -> Result<(), CommandError>,
{
    match body() {
        Ok(()) => Ok(()),
        Err(CommandError::Rendered { code }) => Err(CommandError::Rendered { code }),
        Err(e) => {
            let message = e.to_string();
            let kind = ErrorKind::from(&e);
            let code = e.exit_code_byte();
            print_error(&message, kind);
            Err(CommandError::Rendered { code })
        }
    }
}
