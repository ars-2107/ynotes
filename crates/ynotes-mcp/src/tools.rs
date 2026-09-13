//! Shared explicit-root selection, payload rendering, and error classification.
//! Absent stores return a successful empty answer. Successful tool payloads are
//! provided as both JSON text and structured content.

use std::path::{Path, PathBuf};

use rmcp::model::{CallToolResult, ContentBlock};
use ynotes::Store;

/// The v1 `storeAbsent` payload: a success in a repository with no store.
pub(crate) fn store_absent() -> serde_json::Value {
    serde_json::json!({"store": "absent"})
}

/// Validate the explicit repository selection shared by every MCP tool.
/// Requiring an absolute directory means a long-lived server never consults
/// its ambient process cwd to decide which repository a request addresses.
///
/// # Errors
///
/// A usage-class message when `root` is not an absolute existing directory.
pub(crate) fn explicit_root(root: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(root);
    if !path.is_absolute() {
        return Err("`root` must be an absolute repository path".to_owned());
    }
    if !path.is_dir() {
        return Err(format!("`root` is not a directory: {}", path.display()));
    }
    Ok(path)
}

/// Resolve a tool file argument against the selected repository root. Absolute
/// file paths remain absolute; relative paths are never interpreted against the
/// server cwd.
pub(crate) fn file_for_root(root: &Path, file: &str) -> PathBuf {
    let path = PathBuf::from(file);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

/// Open the store directly beneath the selected repository root. No ancestor
/// walk is permitted: explicit repository selection must not fall through to a
/// parent workspace's store. A repository that never adopted ynotes is a
/// successful empty answer, not a failure.
///
/// # Errors
///
/// Any [`ynotes::Error`] other than `StoreNotFound` (e.g. an unreadable or
/// malformed store), which a caller must surface as an engine error.
pub(crate) fn discover_for(root: &Path) -> Result<Option<Store>, ynotes::Error> {
    Store::open_at_root(root)
}

/// Turn optional `start`/`end` ints into the engine's [`ynotes::LineSpec`].
/// `end` without `start` is a usage error; a bare `start` collapses to a
/// single line. The returned message is the bare reason, the caller tags it
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

/// Compact-JSON success: the contract payload as one text block and,
/// mirrored byte-for-byte in value terms, as `structuredContent`, text for
/// text-only clients, the structured face for schema-aware ones. Compact,
/// not pretty, tool results are token-metered.
pub(crate) fn ok_json(v: &impl serde::Serialize) -> CallToolResult {
    match serde_json::to_value(v) {
        // `structured` renders the text block from the value (`Display` on
        // `Value` is compact) and mirrors it as `structuredContent`.
        Ok(value) => CallToolResult::structured(value),
        // Unreachable for the plain-data payload types; surfaced as a real
        // engine error rather than the old fake-success `{"error": …}` text.
        Err(e) => tool_err("engine", format!("render: {e}")),
    }
}

/// Compact-JSON *failure*: the same contract payload [`ok_json`] renders, but
/// carried on an error result (`isError: true`), for the tools whose payload
/// itself reports the failure (forget's `not_found`/`ambiguous` partition).
/// Rendered through the same `to_value` route as `ok_json`, so the success and
/// failure faces of a payload cannot fork on key order or formatting. Error
/// results stay text-only (no `structuredContent`), matching `tool_err`.
pub(crate) fn err_json(v: &impl serde::Serialize) -> CallToolResult {
    match serde_json::to_value(v) {
        Ok(value) => CallToolResult::error(vec![ContentBlock::text(value.to_string())]),
        Err(e) => tool_err("engine", format!("render: {e}")),
    }
}

/// In-band tool failure. `kind` mirrors the CLI's error classes
/// (`"usage"` | `"engine"` | `"io"`) so agents can branch, and the message
/// carries the remedy, same wording as the CLI.
pub(crate) fn tool_err(kind: &str, msg: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!("{kind}: {msg}"))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_json_mirrors_the_payload_as_text_and_structured_content() {
        let wire = serde_json::to_value(ok_json(&serde_json::json!({"store": "absent"}))).unwrap();
        assert_eq!(wire["content"][0]["text"], "{\"store\":\"absent\"}");
        assert_eq!(
            wire["structuredContent"],
            serde_json::json!({"store": "absent"})
        );
        assert_eq!(wire["isError"], serde_json::json!(false));
    }

    #[test]
    fn ok_json_render_failure_is_an_engine_error() {
        struct Broken;
        impl serde::Serialize for Broken {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("cannot render"))
            }
        }
        let result = ok_json(&Broken);
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
    }

    /// The text block preserves struct declaration order, the load-bearing
    /// effect of `serde_json`'s `preserve_order` feature. Without it, the
    /// `to_value` round-trip in `ok_json` re-sorts keys alphabetically and
    /// the text block's bytes silently diverge from direct `to_string`.
    #[test]
    fn ok_json_text_block_preserves_declaration_order() {
        #[derive(serde::Serialize)]
        struct Probe {
            zebra: u32,
            alpha: u32,
        }
        let wire = serde_json::to_value(ok_json(&Probe { zebra: 1, alpha: 2 })).unwrap();
        assert_eq!(wire["content"][0]["text"], "{\"zebra\":1,\"alpha\":2}");
    }

    #[test]
    fn tool_err_is_text_only() {
        let result = tool_err("usage", "bad input");
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
    }

    #[test]
    fn err_json_carries_the_payload_text_on_a_text_only_error_result() {
        #[derive(serde::Serialize)]
        struct Probe {
            zebra: u32,
            alpha: u32,
        }
        let result = err_json(&Probe { zebra: 1, alpha: 2 });
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["content"][0]["text"], "{\"zebra\":1,\"alpha\":2}");
    }

    #[test]
    fn explicit_root_rejects_cwd_relative_repository_selection() {
        let error = explicit_root(".").unwrap_err();
        assert!(error.contains("absolute repository path"));
    }
}
