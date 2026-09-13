//! `recall`, the loop's read: notes overlapping a file or region, resolved
//! against the current code (rename-following included, invariant #8 pure).

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::{self, MalformedView, NoteView, QueryCountData, QueryData, QuerySpecView};

use crate::tools::{
    discover_for, explicit_root, file_for_root, line_spec, ok_json, store_absent, tool_err,
};

/// Arguments for the `recall` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version, it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct RecallArgs {
    /// Absolute path to the repository root this request addresses.
    pub(crate) root: String,
    /// Path to the file, relative to the repo root or absolute.
    pub(crate) file: String,
    /// 1-based first line; omit with `end` for the whole file.
    #[schemars(range(min = 1))]
    pub(crate) start: Option<u32>,
    /// 1-based last line; needs `start`; omit for a single line.
    #[schemars(range(min = 1))]
    pub(crate) end: Option<u32>,
    /// Return only the status breakdown (cheap existence check).
    pub(crate) count: Option<bool>,
}

/// Resolve the notes overlapping `args` and render them as a contract payload.
///
/// A read only: it discovers the store, resolves every note for the file
/// against the current code (following renames, per invariant #8, without ever
/// writing), and serialises the v1 `queryData`/`queryCountData` payload. A repo
/// with no store answers `{"store":"absent"}`, a success, not an error, so an
/// ambient server does not error-spam repositories that never adopted ynotes.
pub(crate) fn run(args: &RecallArgs) -> CallToolResult {
    let root = match explicit_root(&args.root) {
        Ok(root) => root,
        Err(message) => return tool_err("usage", message),
    };
    let file = file_for_root(&root, &args.file);
    let store = match discover_for(&root) {
        Ok(Some(s)) => s,
        Ok(None) => return ok_json(&store_absent()),
        Err(e) => return tool_err("engine", e),
    };
    let spec = match line_spec(args.start, args.end) {
        Ok(s) => s,
        Err(msg) => return tool_err("usage", msg),
    };
    let result = match ynotes::query(&store, &file, spec) {
        Ok(r) => r,
        Err(e @ ynotes::Error::OutsideStore { .. }) => return tool_err("usage", e),
        Err(e) => return tool_err("engine", e),
    };
    let mut warnings: Vec<String> = Vec::new();
    if !file.exists() {
        warnings.push(format!(
            "`{}` does not exist (check the path)",
            file.display()
        ));
    }
    if let Some(advisory) = result.index.advisory() {
        warnings.push(advisory);
    }
    let at = match (args.start, args.end) {
        (Some(s), Some(e)) => format!("{s}:{e}"),
        (Some(s), None) => s.to_string(),
        _ => String::new(),
    };
    let query = QuerySpecView {
        file: file.to_string_lossy().into_owned(),
        at,
    };
    if args.count.unwrap_or(false) {
        let count = contract::count_view(result.matched.iter().chain(result.orphaned.iter()));
        return ok_json(&QueryCountData {
            query,
            count,
            malformed: result.malformed.len(),
            index: contract::index_health_view(result.index),
            warnings,
        });
    }
    let notes: Vec<NoteView> = result
        .matched
        .iter()
        .chain(result.orphaned.iter())
        .map(|n| contract::note_view(n, false))
        .collect();
    let malformed: Vec<MalformedView> = result
        .malformed
        .iter()
        .map(contract::malformed_view)
        .collect();
    ok_json(&QueryData {
        query,
        notes,
        malformed,
        index: contract::index_health_view(result.index),
        warnings,
    })
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
    /// `body`. Returns the tempdir (kept alive) and the note file's absolute
    /// path.
    fn store_with_note(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
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
        (dir, file)
    }

    fn args_for(
        file: &std::path::Path,
        start: Option<u32>,
        end: Option<u32>,
        count: Option<bool>,
    ) -> RecallArgs {
        RecallArgs {
            root: file.parent().unwrap().to_string_lossy().into_owned(),
            file: file.to_string_lossy().into_owned(),
            start,
            end,
            count,
        }
    }

    #[test]
    fn recall_returns_notes_for_file() {
        let (_dir, file) = store_with_note("deliberate: leave the retry loop");
        let result = run(&args_for(&file, None, None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["notes"][0]["body"], "deliberate: leave the retry loop");
    }

    #[test]
    fn recall_count_mode_totals_one() {
        let (_dir, file) = store_with_note("x");
        let result = run(&args_for(&file, None, None, Some(true)));
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["count"]["total"], 1);
    }

    #[test]
    fn recall_without_store_is_store_absent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "alpha\n").unwrap();
        let result = run(&args_for(&file, None, None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["store"], "absent");
    }

    #[test]
    fn recall_end_without_start_is_usage_error() {
        let (_dir, file) = store_with_note("x");
        let result = run(&args_for(&file, None, Some(3), None));
        assert_eq!(
            result.is_error,
            Some(true),
            "expected a usage error: {result:?}"
        );
        let text = text_of(&result);
        assert!(
            text.starts_with("usage:"),
            "expected `usage:` prefix, got: {text}"
        );
    }

    #[test]
    fn recall_uses_root_to_select_between_repositories() {
        let (first, _) = store_with_note("first repository");
        let (second, _) = store_with_note("second repository");
        let result = run(&RecallArgs {
            root: second.path().to_string_lossy().into_owned(),
            file: "f.txt".to_owned(),
            start: None,
            end: None,
            count: None,
        });
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let value: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(value["notes"][0]["body"], "second repository");
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn recall_does_not_fall_through_to_a_parent_store() {
        let parent = tempfile::tempdir().unwrap();
        Store::init(parent.path()).unwrap();
        let child = parent.path().join("nested-repository");
        std::fs::create_dir(&child).unwrap();
        std::fs::write(child.join("f.txt"), "local\n").unwrap();

        let result = run(&RecallArgs {
            root: child.to_string_lossy().into_owned(),
            file: "f.txt".to_owned(),
            start: None,
            end: None,
            count: None,
        });
        let value: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(value["store"], "absent");
    }
}
