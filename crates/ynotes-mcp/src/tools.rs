//! Helpers every tool shares: store discovery that treats "no store" as a
//! successful empty answer (an ambient server must not error-spam repos that
//! never adopted ynotes), contract-payload serialisation into a text content
//! block, and the usage/engine error split mirroring the CLI's exit classes.

use std::path::Path;

use rmcp::model::{CallToolResult, ContentBlock};
use ynotes::Store;

/// The v12 `storeAbsent` payload: a SUCCESS in a repo with no store.
pub(crate) fn store_absent() -> serde_json::Value {
    serde_json::json!({"store": "absent"})
}

/// Discover the store for a path (or the server cwd), mapping
/// [`ynotes::Error::StoreNotFound`] to `None` rather than an error — a repo
/// that never adopted ynotes is a successful empty answer, not a failure.
///
/// # Errors
///
/// Any [`ynotes::Error`] other than `StoreNotFound` (e.g. an unreadable or
/// malformed store), which a caller must surface as an engine error.
pub(crate) fn discover_for(start: &Path) -> Result<Option<Store>, ynotes::Error> {
    match Store::discover_from(start) {
        Ok(s) => Ok(Some(s)),
        Err(ynotes::Error::StoreNotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Turn optional `start`/`end` ints into the engine's [`ynotes::LineSpec`].
/// `end` without `start` is a usage error; a bare `start` collapses to a
/// single line. The returned message is the bare reason — the caller tags it
/// with the `usage` class (so the class prefix is never doubled).
///
/// # Errors
///
/// A usage-class message when `end` is given without `start`, or when the
/// resulting line/range is not a valid location (a reversed range, line `0`).
pub(crate) fn line_spec(start: Option<u32>, end: Option<u32>) -> Result<ynotes::LineSpec, String> {
    match (start, end) {
        (None, None) => Ok(ynotes::LineSpec::Whole),
        (None, Some(_)) => Err("`end` requires `start`".to_owned()),
        (Some(s), None) => s
            .to_string()
            .parse()
            .map_err(|e: ynotes::Error| e.to_string()),
        (Some(s), Some(e)) => format!("{s}:{e}")
            .parse()
            .map_err(|e: ynotes::Error| e.to_string()),
    }
}

/// Compact-JSON success: the contract payload as one text block. Compact,
/// not pretty — tool results are token-metered.
pub(crate) fn ok_json(v: &impl serde::Serialize) -> CallToolResult {
    let text =
        serde_json::to_string(v).unwrap_or_else(|e| format!("{{\"error\":\"render: {e}\"}}"));
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// In-band tool failure. `kind` mirrors the CLI's error classes
/// (`"usage"` | `"engine"` | `"io"`) so agents can branch, and the message
/// carries the remedy, same wording as the CLI.
pub(crate) fn tool_err(kind: &str, msg: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!("{kind}: {msg}"))])
}
