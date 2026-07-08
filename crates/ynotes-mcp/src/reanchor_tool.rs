//! `reanchor` — the loop's sole resolve-time write path: persist re-anchors for
//! every note that confidently moved, follow committed renames, collapse merge
//! duplicates.
//!
//! A thin wrapper over [`ynotes::reanchor`]: all policy — which notes to
//! refresh, the refusal to touch an orphaned note, invariant #8's "a resolve
//! never writes; only reanchor does" — lives in the engine. This module only
//! discovers the store, invokes the pass, and renders the contract payload. It
//! adds no write logic of its own. A repo with no store answers
//! `{"store":"absent"}` (a success, per §5.0), so the ambient server never
//! error-spams repositories that never adopted ynotes.

use std::path::Path;

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::ReanchorView;

use crate::tools::{discover_for, ok_json, store_absent, tool_err};

/// Arguments for the `reanchor` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version — it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct ReanchorArgs {
    /// Preview the pass without writing anything.
    pub(crate) dry_run: Option<bool>,
}

/// Run (or, with `dry_run`, preview) the re-anchor pass and render its report as
/// a v12 `reanchorData` payload.
///
/// Discovers the store from the server cwd and delegates the whole pass to
/// [`ynotes::reanchor`], which owns every policy decision (invariant #8). A repo
/// with no store answers `{"store":"absent"}` — a success, not an error, so an
/// ambient server does not error-spam repositories that never adopted ynotes.
pub(crate) fn run(args: &ReanchorArgs) -> CallToolResult {
    let store = match discover_for(Path::new(".")) {
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

    /// Restores the process cwd on drop (see the note in `forget`'s tests).
    #[derive(Debug)]
    struct CwdGuard(std::path::PathBuf);

    impl CwdGuard {
        fn enter(dir: &Path) -> Self {
            let prev = std::env::current_dir().expect("read cwd");
            std::env::set_current_dir(dir).expect("enter cwd");
            Self(prev)
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            drop(std::env::set_current_dir(&self.0));
        }
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
        let _cwd = CwdGuard::enter(dir.path());

        let result = run(&ReanchorArgs {
            dry_run: Some(true),
        });
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["dry_run"], true);
        // The region really did move, so the pass *would* rewrite the note — the
        // assertion below is non-vacuous: it proves the dry run declined a write
        // it was about to make, not merely that nothing needed doing.
        assert!(
            !v["changed"].as_array().unwrap().is_empty(),
            "the drifted note should be reported as changed: {v}"
        );

        // The note keeps its original id on disk — the dry run wrote nothing.
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
        let _cwd = CwdGuard::enter(dir.path());
        let result = run(&ReanchorArgs { dry_run: None });
        assert_ne!(
            result.is_error,
            Some(true),
            "store-absent must be a success: {result:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["store"], "absent");
    }
}
