//! `remember` — the loop's write: save (or supersede) a note anchored to a
//! region. Auto-creates the store at the git root on first use (§4 of the
//! design: adoption must be zero-ceremony).

use std::path::{Path, PathBuf};

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::SaveData;
use ynotes::{Note, SelectorBundle, SourceFile, Store};

use crate::tools::{line_spec, ok_json, tool_err};

/// Arguments for the `remember` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version — it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct RememberArgs {
    /// Path to the file the context is about.
    pub(crate) file: String,
    /// 1-based first line; omit both bounds for a file-scoped note.
    #[schemars(range(min = 1))]
    pub(crate) start: Option<u32>,
    /// 1-based last line; needs `start`.
    #[schemars(range(min = 1))]
    pub(crate) end: Option<u32>,
    /// The region's text, exactly as the file reads now. Verifies `start` and
    /// corrects it when the line numbers went stale, so the note cannot land
    /// on the wrong region. Requires `start`; the region's length comes from
    /// `end` when given, else from the quote's line count.
    pub(crate) code: Option<String>,
    /// The context text. Write for a reader who has not seen this session.
    pub(crate) body: String,
}

/// Save (or supersede) the note `args` describes and render its identity as a
/// v12 `saveData` payload.
///
/// The sole MCP write path, and the only tool allowed to create a store: when
/// none exists it bootstraps one at the enclosing git root via
/// [`Store::discover_or_init_from`] (the zero-ceremony adoption path), reporting
/// `store_created`. Discovery starts from the target file's own directory, not
/// the server's cwd, so the right store is found whatever directory the client
/// launched the server in. A repo with no git root cannot place a store
/// deterministically, so it is a usage error carrying the engine's own
/// `ynotes init` remedy — passed through unchanged, never re-worded.
pub(crate) fn run(args: &RememberArgs) -> CallToolResult {
    if args.body.trim().is_empty() {
        return tool_err("usage", "note body must not be empty");
    }
    let file = PathBuf::from(&args.file);
    let anchor = std::path::absolute(&file).unwrap_or_else(|_| file.clone());
    let search_from = anchor.parent().unwrap_or(Path::new(".")).to_path_buf();
    let (store, store_created) = match Store::discover_or_init_from(&search_from) {
        Ok(pair) => pair,
        Err(e @ ynotes::Error::StoreNotFound { .. }) => return tool_err("usage", e),
        Err(e) => return tool_err("engine", e),
    };
    if let Err(e) = store.reject_symlink_escape(&file) {
        return tool_err("usage", e);
    }
    let target = match store.relativize(&file) {
        Ok(t) => t,
        Err(e) => return tool_err("usage", e),
    };
    let source = match SourceFile::read(&file) {
        Ok(s) => s,
        Err(e) => return tool_err("engine", e),
    };
    let spec = match line_spec(args.start, args.end) {
        Ok(s) => s,
        Err(msg) => return tool_err("usage", msg),
    };
    // With `code`, the engine verifies the coordinates against the file
    // (correcting stale ones) before anchoring; its refusals carry their own
    // recovery text, passed through unchanged.
    let resolved = match args.code.as_deref() {
        Some(code) => ynotes::resolve_scope_verified(spec, code, &source),
        None => ynotes::resolve_scope(spec, &source),
    };
    let (scope, range) = match resolved {
        Ok(pair) => pair,
        Err(e) => return tool_err("usage", e),
    };
    let bundle = match SelectorBundle::capture_full(&source, range) {
        Ok(b) => b,
        Err(e) => return tool_err("engine", e),
    };
    let note = match Note::new(
        target.clone(),
        scope,
        bundle,
        args.body.trim_end().to_owned(),
    ) {
        Ok(n) => n,
        Err(e) => return tool_err("engine", e),
    };
    let (created, _superseded) = match store.save_superseding(&note) {
        Ok(pair) => pair,
        Err(e) => return tool_err("engine", e),
    };
    ok_json(&SaveData {
        id: note.id.clone(),
        target,
        scope: ynotes::contract::scope_word_json(scope).to_owned(),
        range: [range.start(), range.end()],
        created,
        store_created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract the first text content block from a tool result.
    fn text_of(r: &CallToolResult) -> String {
        r.content[0]
            .as_text()
            .expect("text content block")
            .text
            .clone()
    }

    fn args_for(file: &Path, start: Option<u32>, end: Option<u32>, body: &str) -> RememberArgs {
        RememberArgs {
            file: file.to_string_lossy().into_owned(),
            start,
            end,
            code: None,
            body: body.to_owned(),
        }
    }

    fn args_with_code(
        file: &Path,
        start: Option<u32>,
        end: Option<u32>,
        code: &str,
        body: &str,
    ) -> RememberArgs {
        RememberArgs {
            file: file.to_string_lossy().into_owned(),
            start,
            end,
            code: Some(code.to_owned()),
            body: body.to_owned(),
        }
    }

    /// A tempdir with a `.git/` directory (so `discover_or_init_from` treats it
    /// as a repo root) and a `f.txt` of three lines, but no store yet.
    fn git_repo_no_store() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "alpha\nbravo\ncharlie\n").unwrap();
        (dir, file)
    }

    #[test]
    fn remember_bootstraps_store_at_git_root() {
        let (dir, file) = git_repo_no_store();
        let result = run(&args_for(&file, Some(1), Some(2), "why bravo matters"));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["store_created"], true);
        assert!(
            dir.path().join(".ynotes").is_dir(),
            ".ynotes store not created"
        );
    }

    #[test]
    fn remember_is_idempotent_by_content_address() {
        let (_dir, file) = git_repo_no_store();
        let first = run(&args_for(&file, Some(1), Some(2), "why bravo matters"));
        let v1: serde_json::Value = serde_json::from_str(&text_of(&first)).unwrap();
        assert_eq!(v1["created"], true, "first save should create");
        assert_eq!(v1["store_created"], true, "first save should bootstrap");

        let second = run(&args_for(&file, Some(1), Some(2), "why bravo matters"));
        let v2: serde_json::Value = serde_json::from_str(&text_of(&second)).unwrap();
        assert_eq!(v2["created"], false, "identical second save is idempotent");
        assert_eq!(v2["store_created"], false, "store already existed");
    }

    #[test]
    fn remember_without_git_root_errors_with_init_remedy() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "alpha\n").unwrap();
        let result = run(&args_for(&file, None, None, "some context"));
        assert_eq!(result.is_error, Some(true), "expected an error: {result:?}");
        let text = text_of(&result);
        assert!(
            text.contains("ynotes init"),
            "expected the `ynotes init` remedy, got: {text}"
        );
    }

    #[test]
    fn remember_line_zero_is_usage_error() {
        let (_dir, file) = git_repo_no_store();
        let result = run(&args_for(&file, Some(0), None, "some context"));
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
        assert!(
            text.contains("1-based"),
            "expected the core line-0 wording, got: {text}"
        );
    }

    #[test]
    fn remember_empty_body_is_usage_error() {
        let (_dir, file) = git_repo_no_store();
        let result = run(&args_for(&file, None, None, "   "));
        assert_eq!(
            result.is_error,
            Some(true),
            "expected a usage error: {result:?}"
        );
        assert_eq!(text_of(&result), "usage: note body must not be empty");
    }

    #[test]
    fn remember_with_stale_start_relocates_and_reports_the_corrected_range() {
        let (_dir, file) = git_repo_no_store();
        std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();
        let result = run(&args_with_code(&file, Some(2), None, "five\nsix", "why"));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        // The corrected location rides the payload's existing `range` field.
        assert_eq!(v["range"], serde_json::json!([5, 6]));
        assert_eq!(v["scope"], "range");
    }

    #[test]
    fn remember_code_not_found_carries_the_reread_remedy() {
        let (_dir, file) = git_repo_no_store();
        let result = run(&args_with_code(&file, Some(1), None, "missing text", "why"));
        assert_eq!(result.is_error, Some(true), "expected an error: {result:?}");
        let text = text_of(&result);
        assert!(
            text.starts_with("usage:"),
            "expected `usage:` prefix, got: {text}"
        );
        assert!(
            text.contains("re-read the file"),
            "expected the recovery, got: {text}"
        );
    }

    #[test]
    fn remember_ambiguous_code_lists_every_candidate() {
        let (_dir, file) = git_repo_no_store();
        std::fs::write(&file, "dup\nx\ndup\ny\n").unwrap();
        let result = run(&args_with_code(&file, Some(4), None, "dup", "why"));
        assert_eq!(result.is_error, Some(true), "expected an error: {result:?}");
        let text = text_of(&result);
        assert!(
            text.starts_with("usage:"),
            "expected `usage:` prefix, got: {text}"
        );
        assert!(
            text.contains("lines 1, 3"),
            "expected the candidates, got: {text}"
        );
        assert!(
            text.contains("start matches none of them"),
            "expected the diagnosis, got: {text}"
        );
    }

    #[test]
    fn remember_code_without_start_is_a_usage_error() {
        let (_dir, file) = git_repo_no_store();
        let result = run(&args_with_code(&file, None, None, "alpha", "why"));
        assert_eq!(result.is_error, Some(true), "expected an error: {result:?}");
        let text = text_of(&result);
        assert!(
            text.contains("requires a starting line"),
            "expected the whole-file refusal, got: {text}"
        );
    }
}
