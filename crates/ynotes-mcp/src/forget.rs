//! `forget`, the loop's targeted delete: remove one note by id or hex prefix.
//!
//! The single-id port of `delete`'s engine interaction. A resolved-and-removed
//! note is a success; a request that resolves to nothing (`not_found`) or to
//! more than one note (`ambiguous`) is surfaced as an *error* result whose text
//! is the same machine-readable partition, never swallowed (invariant #4). A
//! corrupt record is removable only by its exact 64-character id, mirroring the
//! CLI's `purge_unreadable` path.

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::IdMatch;
use ynotes::contract::{AmbiguousView, DeleteData, DeletedView, body_excerpt, scope_word_json};

use crate::tools::{discover_for, err_json, explicit_root, ok_json, tool_err};

/// Arguments for the `forget` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version, it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct ForgetArgs {
    /// Absolute path to the repository root this request addresses.
    pub(crate) root: String,
    /// Full 64-char note id, or an unambiguous hex prefix of >= 4 characters
    /// (recall/notes results carry full ids).
    pub(crate) id: String,
    /// Preview without deleting.
    pub(crate) dry_run: Option<bool>,
}

/// Resolve `args.id` to a note and remove it (unless previewing), rendering the
/// outcome as a v1 `deleteData` payload.
///
/// The single-request port of `delete`: validate the id, discover the store,
/// then match the id to `One`/`None`/`Ambiguous`. `One` removes the note (unless
/// `dry_run`); a `None` that names an unreadable record by its exact id purges it
/// (a success); everything else lands in `not_found` or `ambiguous`. A request
/// that failed to delete anything (`not_found`/`ambiguous`) is returned as an
/// *error* result carrying the same `deleteData` JSON, so an agent gets the
/// partition either way and it is never silently dropped (invariant #4). Unlike
/// `notes`, a store-less repo is a surfaced error (a caller bug, there is
/// nothing to forget), not a `store_absent` success.
pub(crate) fn run(args: &ForgetArgs) -> CallToolResult {
    if let Err(e) = ynotes::validate_id_prefix(&args.id) {
        return tool_err("usage", e);
    }
    let root = match explicit_root(&args.root) {
        Ok(root) => root,
        Err(message) => return tool_err("usage", message),
    };
    let store = match discover_for(&root) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return tool_err(
                "usage",
                "no ynotes store in this repository; nothing to forget",
            );
        }
        Err(e) => return tool_err("engine", e),
    };
    let dry_run = args.dry_run.unwrap_or(false);

    let mut deleted: Vec<DeletedView> = Vec::new();
    let mut deleted_unreadable: Vec<String> = Vec::new();
    let mut ambiguous: Vec<AmbiguousView> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    match store.match_id_prefix(&args.id) {
        // No readable match. The one removable corrupt case is an exact id
        // naming an unreadable record: `purge_unreadable` is guarded to the
        // exact id and refuses a readable note, so this never bypasses a real
        // delete. Anything else stays `not_found`.
        Ok(IdMatch::None) => match store.purge_unreadable(&args.id, dry_run) {
            Ok(true) => deleted_unreadable.push(args.id.clone()),
            Ok(false) => not_found.push(args.id.clone()),
            Err(e) => return tool_err("engine", e),
        },
        Ok(IdMatch::One(only)) => {
            let view = DeletedView {
                requested: args.id.clone(),
                id: only.id.clone(),
                target: only.target.clone(),
                scope: scope_word_json(only.scope),
                range: [
                    only.bundle.position.range.start(),
                    only.bundle.position.range.end(),
                ],
                body_excerpt: body_excerpt(&only.body),
            };
            if !dry_run && let Err(e) = store.remove(&only) {
                return tool_err("engine", e);
            }
            deleted.push(view);
        }
        Ok(IdMatch::Ambiguous(many)) => ambiguous.push(AmbiguousView {
            requested: args.id.clone(),
            candidates: many.iter().map(|n| n.id.clone()).collect(),
        }),
        Err(e) => return tool_err("engine", e),
    }

    let had_failure = !ambiguous.is_empty() || !not_found.is_empty();
    let data = DeleteData {
        requested: 1,
        deleted,
        deleted_unreadable,
        ambiguous,
        not_found,
        dry_run,
    };

    if had_failure {
        // The single request could not be satisfied. Surface the same machine
        // partition as an error result (`isError: true`) so an agent branches
        // on it without parsing prose, rendered through the same route as
        // `ok_json`, so the success and failure faces cannot fork.
        err_json(&data)
    } else {
        ok_json(&data)
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

    fn args_for(root: &std::path::Path, id: &str, dry_run: Option<bool>) -> ForgetArgs {
        ForgetArgs {
            root: root.to_string_lossy().into_owned(),
            id: id.to_owned(),
            dry_run,
        }
    }

    #[test]
    fn forget_deletes_by_unique_prefix_and_empties_store() {
        let (dir, id) = store_with_note("obsolete note");
        let result = run(&args_for(dir.path(), &id[..12], None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["deleted"][0]["id"], id);
        assert_eq!(v["dry_run"], false);
        let store = Store::discover_from(dir.path()).unwrap();
        assert!(
            store.all_notes().unwrap().notes.is_empty(),
            "store should be empty after a real delete"
        );
    }

    #[test]
    fn forget_missing_prefix_is_error_naming_not_found() {
        let (dir, _id) = store_with_note("keep");
        let result = run(&args_for(dir.path(), "deadbeef", None));
        assert_eq!(
            result.is_error,
            Some(true),
            "a failed request must be an error: {result:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["not_found"][0], "deadbeef");
        assert_eq!(v["deleted"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn forget_dry_run_leaves_the_note_present() {
        let (dir, id) = store_with_note("keep me for now");
        let result = run(&args_for(dir.path(), &id, Some(true)));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["dry_run"], true);
        assert_eq!(v["deleted"][0]["id"], id);
        let store = Store::discover_from(dir.path()).unwrap();
        assert_eq!(
            store.all_notes().unwrap().notes.len(),
            1,
            "a dry run must not write"
        );
    }

    #[test]
    fn forget_without_store_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = run(&args_for(dir.path(), "deadbeef", None));
        assert_eq!(
            result.is_error,
            Some(true),
            "forgetting in a store-less repo is a surfaced error: {result:?}"
        );
        let text = text_of(&result);
        assert!(
            text.starts_with("usage:"),
            "expected a usage-class error, got: {text}"
        );
    }

    #[test]
    fn forget_purges_a_corrupt_record_by_exact_id() {
        let (dir, id) = store_with_note("about to be corrupted");
        // Corrupt the record on disk: notes fan out to
        // `.ynotes/notes/<first-2>/<rest>.json`.
        let record = dir
            .path()
            .join(".ynotes/notes")
            .join(&id[..2])
            .join(format!("{}.json", &id[2..]));
        std::fs::write(&record, "garbage").unwrap();
        let result = run(&args_for(dir.path(), &id, None));
        assert_ne!(
            result.is_error,
            Some(true),
            "a purge is a success: {result:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["deleted_unreadable"][0], id);
        assert!(!record.exists(), "the corrupt record file was removed");
    }
}
