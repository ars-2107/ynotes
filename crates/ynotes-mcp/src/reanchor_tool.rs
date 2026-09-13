//! Adapt [`ynotes::reanchor`] to MCP and render the shared report.
//! An absent store returns a successful `storeAbsent` payload without creating one.

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::ReanchorView;

use crate::tools::{discover_for, explicit_root, ok_json, store_absent, tool_err};

/// Arguments for the `reanchor` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version, it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct ReanchorArgs {
    /// Absolute path to the repository root this request addresses.
    pub(crate) root: String,
    /// Preview the pass without writing anything.
    pub(crate) dry_run: Option<bool>,
}

/// Run (or, with `dry_run`, preview) the re-anchor pass and render its report as
/// a v1 `reanchorData` payload.
///
/// Opens the store beneath the explicitly selected root and delegates the whole pass to
/// [`ynotes::reanchor`], which owns every policy decision (invariant #8). A repo
/// with no store answers `{"store":"absent"}`, a success, not an error, so an
/// ambient server does not error-spam repositories that never adopted ynotes.
pub(crate) fn run(args: &ReanchorArgs) -> CallToolResult {
    let root = match explicit_root(&args.root) {
        Ok(root) => root,
        Err(message) => return tool_err("usage", message),
    };
    let store = match discover_for(&root) {
        Ok(Some(s)) => s,
        Ok(None) => return ok_json(&store_absent()),
        Err(e) => return tool_err("engine", e),
    };
    let dry_run = args.dry_run.unwrap_or(false);
    match ynotes::reanchor(&store, dry_run) {
        Ok(report) => ok_json(&ReanchorView::from(&report)),
        Err(e) => tool_err("engine", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ynotes::{LineRange, Note, Scope, SelectorBundle, SourceFile, Store};

    /// Extract the first text content block from a tool result.
    fn text_of(r: &CallToolResult) -> String {
        r.content[0]
            .as_text()
            .expect("text content block")
            .text
            .clone()
    }

    /// A tempdir store with one Range note over lines 2-3 of `f.txt`, body
    /// `body`. Returns the tempdir (kept alive) and the note's id.
    fn store_with_note(body: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "alpha\nbravo\ncharlie\ndelta\n").unwrap();
        let src = SourceFile::read(&file).unwrap();
        let bundle = SelectorBundle::capture_full(&src, LineRange::new(2, 3).unwrap()).unwrap();
        let note = Note::new(
            store.relativize(&file).unwrap(),
            Scope::Range,
            bundle,
            body.to_owned(),
        )
        .unwrap();
        store.save(&note).unwrap();
        (dir, note.id)
    }

    #[test]
    fn reanchor_dry_run_leaves_note_ids_unchanged() {
        let (dir, id) = store_with_note("keep this anchored");
        // Prepend a line so the region drifts down: `reanchor` would refresh it,
        // exercising the write path a dry run must decline to take.
        std::fs::write(
            dir.path().join("f.txt"),
            "inserted\nalpha\nbravo\ncharlie\ndelta\n",
        )
        .unwrap();
        let result = run(&ReanchorArgs {
            root: dir.path().to_string_lossy().into_owned(),
            dry_run: Some(true),
        });
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["dry_run"], true);
        // The region really did move, so the pass *would* rewrite the note, the
        // assertion below is non-vacuous: it proves the dry run declined a write
        // it was about to make, not merely that nothing needed doing.
        assert!(
            !v["changed"].as_array().unwrap().is_empty(),
            "the drifted note should be reported as changed: {v}"
        );

        // The note keeps its original id on disk, the dry run wrote nothing.
        let store = Store::discover_from(dir.path()).unwrap();
        let ids: Vec<String> = store
            .all_notes()
            .unwrap()
            .notes
            .iter()
            .map(|n| n.id.clone())
            .collect();
        assert_eq!(ids, vec![id], "a dry run must not rotate or rewrite an id");
    }

    #[test]
    fn reanchor_without_store_is_store_absent() {
        let dir = tempfile::tempdir().unwrap();
        let result = run(&ReanchorArgs {
            root: dir.path().to_string_lossy().into_owned(),
            dry_run: None,
        });
        assert_ne!(
            result.is_error,
            Some(true),
            "store-absent must be a success: {result:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["store"], "absent");
    }
}
