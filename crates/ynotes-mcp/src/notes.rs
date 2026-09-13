//! Read-only lookup by ID, body substring, or whole-store inventory.
//! The `id`, `body_contains`, and `count` modes are mutually exclusive.
//! An absent store returns a successful `storeAbsent` payload.

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::{
    ListCountData, ListData, LookupData, ShowData, count_view, index_health_view, malformed_view,
    note_view,
};
use ynotes::{IdMatch, Store};

use crate::tools::{discover_for, explicit_root, ok_json, store_absent, tool_err};

/// Arguments for the `notes` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version, it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct NotesArgs {
    /// Absolute path to the repository root this request addresses.
    pub(crate) root: String,
    /// Fetch exactly one note: full id or unambiguous >= 4-char hex prefix.
    pub(crate) id: Option<String>,
    /// Search note bodies for this substring (case-sensitive).
    pub(crate) body_contains: Option<String>,
    /// Whole-store status breakdown instead of note bodies.
    pub(crate) count: Option<bool>,
}

/// Dispatch a `notes` request to the right read and render its contract payload.
///
/// `id`, `body_contains`, and `count` name mutually exclusive shapes, so a
/// request that sets more than one is a usage error, checked before the store
/// is touched, so it does not depend on store state. Otherwise: `id` resolves a
/// single note against its stored target (as `show` does) into `showData`;
/// `body_contains` runs `lookup` into `lookupData`; neither runs `list` into
/// rename-aware `listData`, or a `count` breakdown into `listCountData`. A repo with no store
/// answers `{"store":"absent"}`, a pure read never errors on adoption.
pub(crate) fn run(args: &NotesArgs) -> CallToolResult {
    let count = args.count.unwrap_or(false);
    // The three selectors are mutually exclusive; valid requests set at most
    // one. Reject anything that sets two or more, naming the valid forms.
    let selectors = [args.id.is_some(), args.body_contains.is_some(), count];
    if selectors.iter().filter(|&&set| set).count() > 1 {
        return tool_err(
            "usage",
            "pass exactly one of: id | body_contains | count | no params (full list)",
        );
    }

    let root = match explicit_root(&args.root) {
        Ok(root) => root,
        Err(message) => return tool_err("usage", message),
    };
    let store = match discover_for(&root) {
        Ok(Some(s)) => s,
        Ok(None) => return ok_json(&store_absent()),
        Err(e) => return tool_err("engine", e),
    };

    if let Some(id) = &args.id {
        return show_one(&store, id);
    }
    if let Some(needle) = &args.body_contains {
        return match ynotes::lookup(&store, None, Some(needle.as_str())) {
            Ok(result) => ok_json(&LookupData {
                notes: result.notes.iter().map(|n| note_view(n, false)).collect(),
            }),
            Err(e) => tool_err("engine", e),
        };
    }

    match ynotes::list(&store, None) {
        Ok(result) if count => ok_json(&ListCountData {
            count: count_view(result.notes.iter()),
            malformed: result.malformed.len(),
            index: index_health_view(result.index),
            warnings: result.index.advisory().into_iter().collect(),
        }),
        Ok(result) => ok_json(&ListData {
            notes: result.notes.iter().map(|n| note_view(n, false)).collect(),
            malformed: result.malformed.iter().map(malformed_view).collect(),
            index: index_health_view(result.index),
            warnings: result.index.advisory().into_iter().collect(),
        }),
        Err(e) => tool_err("engine", e),
    }
}

/// Resolve one note by id or hex prefix and render it as `showData`, mirroring
/// `commands/show.rs`: match the prefix, then anchor the note against its stored
/// target (a missing target file yields an empty source, so the note orphans and
/// is still shown, never a hard failure). `None`/`Ambiguous` become usage errors
/// naming the remedy, exactly as the CLI's shared id resolver does.
fn show_one(store: &Store, id: &str) -> CallToolResult {
    if let Err(e) = ynotes::validate_id_prefix(id) {
        return tool_err("usage", e);
    }
    let note = match store.match_id_prefix(id) {
        Ok(IdMatch::One(note)) => note,
        Ok(IdMatch::None) => {
            return tool_err(
                "usage",
                format!("no note matches `{id}` (use notes with no params to see available ids)"),
            );
        }
        Ok(IdMatch::Ambiguous(candidates)) => {
            return tool_err("usage", ynotes::ambiguous_id_message(id, &candidates));
        }
        Err(e) => return tool_err("engine", e),
    };

    let workdir = match store.workdir() {
        Ok(w) => w.to_path_buf(),
        Err(e) => return tool_err("engine", e),
    };
    let abs = workdir.join(&note.target);

    // The engine resolves this the way the inventory form does, rename
    // corroboration included. Assembling it here instead is how the two shapes
    // of one tool came to disagree: after a `git mv`, the inventory reported
    // the note anchored at its new path while a by-id read of the *same* note
    // reported `orphaned` (invariant #10).
    let resolved = match ynotes::resolve_one(store, note) {
        Ok(resolved) => resolved,
        Err(e) => return tool_err("engine", e),
    };

    let mut warnings = Vec::new();
    // A renamed file is absent at its stored target yet plainly present at the
    // new one, so this advisory would be actively wrong there.
    if !abs.exists() && resolved.relocated_to.is_none() {
        warnings.push(format!(
            "`{}` does not exist (the note's target file is gone)",
            resolved.note.target
        ));
    }
    ok_json(&ShowData {
        note: note_view(&resolved, false),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;
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

    fn args(
        root: &std::path::Path,
        id: Option<&str>,
        body_contains: Option<&str>,
        count: Option<bool>,
    ) -> NotesArgs {
        NotesArgs {
            root: root.to_string_lossy().into_owned(),
            id: id.map(ToOwned::to_owned),
            body_contains: body_contains.map(ToOwned::to_owned),
            count,
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn notes_no_params_lists_all() {
        let (dir, _id) = store_with_note("first note");
        let result = run(&args(dir.path(), None, None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["notes"][0]["body"], "first note");
    }

    #[test]
    fn notes_inventory_surfaces_a_pending_rename_without_persisting_it() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.name", "ynotes test"]);
        git(
            dir.path(),
            &["config", "user.email", "ynotes@example.invalid"],
        );
        let old = dir.path().join("old.rs");
        std::fs::write(&old, "one\nfocus\nthree\nfour\n").unwrap();
        git(dir.path(), &["add", "old.rs"]);
        git(dir.path(), &["commit", "-qm", "base"]);

        let store = Store::init(dir.path()).unwrap();
        let source = SourceFile::read(&old).unwrap();
        let bundle = SelectorBundle::capture_full(&source, LineRange::new(2, 2).unwrap()).unwrap();
        let note = Note::new(
            "old.rs".to_owned(),
            Scope::Range,
            bundle,
            "why focus matters".to_owned(),
        )
        .unwrap();
        store.save(&note).unwrap();

        let new = dir.path().join("new.rs");
        std::fs::rename(&old, &new).unwrap();
        git(dir.path(), &["add", "-A"]);

        let result = run(&args(dir.path(), None, None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let value: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        let note = &value["notes"][0];
        assert_eq!(note["target"], "old.rs");
        assert_eq!(note["relocated_from"], "old.rs");
        assert_eq!(note["relocated_to"], "new.rs");

        let stored = store.all_notes().unwrap();
        assert_eq!(stored.notes.len(), 1);
        assert_eq!(
            stored.notes[0].target, "old.rs",
            "inventory reads must not persist"
        );
    }

    /// One tool, two shapes, one answer. The by-id form used to assemble its
    /// own resolution without rename corroboration, so after a `git mv` the
    /// inventory reported the note anchored at its new path while a by-id read
    /// of the same note reported `orphaned`, a flat contradiction, and the
    /// strongest "your context is lost" signal there is (invariant #10).
    #[test]
    fn a_by_id_read_agrees_with_the_inventory_across_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.name", "ynotes test"]);
        git(
            dir.path(),
            &["config", "user.email", "ynotes@example.invalid"],
        );
        let old = dir.path().join("old.rs");
        std::fs::write(&old, "one\nfocus\nthree\nfour\n").unwrap();
        git(dir.path(), &["add", "old.rs"]);
        git(dir.path(), &["commit", "-qm", "base"]);

        let store = Store::init(dir.path()).unwrap();
        let source = SourceFile::read(&old).unwrap();
        let bundle = SelectorBundle::capture_full(&source, LineRange::new(2, 2).unwrap()).unwrap();
        let note = Note::new(
            "old.rs".to_owned(),
            Scope::Range,
            bundle,
            "why focus matters".to_owned(),
        )
        .unwrap();
        store.save(&note).unwrap();

        std::fs::rename(&old, dir.path().join("new.rs")).unwrap();
        git(dir.path(), &["add", "-A"]);

        let inventory = run(&args(dir.path(), None, None, None));
        let inventory: serde_json::Value = serde_json::from_str(&text_of(&inventory)).unwrap();
        let listed = &inventory["notes"][0];

        let by_id = run(&args(dir.path(), Some(&note.id), None, None));
        assert_ne!(by_id.is_error, Some(true), "unexpected error: {by_id:?}");
        let by_id: serde_json::Value = serde_json::from_str(&text_of(&by_id)).unwrap();
        let single = &by_id["note"];

        assert_eq!(
            single["status"], listed["status"],
            "the two shapes must not disagree about the same note"
        );
        assert_eq!(single["relocated_to"], "new.rs");
        assert!(
            by_id["warnings"].as_array().is_none_or(|w| w
                .iter()
                .all(|m| !m.as_str().unwrap_or_default().contains("does not exist"))),
            "a renamed file is present at its new path: {by_id}"
        );
    }

    #[test]
    fn notes_count_mode_reports_breakdown() {
        let (dir, _id) = store_with_note("x");
        let result = run(&args(dir.path(), None, None, Some(true)));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["count"]["total"], 1);
        assert!(v.get("notes").is_none(), "count mode omits the notes array");
    }

    #[test]
    fn notes_by_id_shows_one_note() {
        let (dir, id) = store_with_note("the specific one");
        let result = run(&args(dir.path(), Some(&id), None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["note"]["id"], id);
        assert_eq!(v["note"]["body"], "the specific one");
    }

    #[test]
    fn notes_by_unique_prefix_shows_one_note() {
        let (dir, id) = store_with_note("found by prefix");
        let result = run(&args(dir.path(), Some(&id[..12]), None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["note"]["id"], id);
    }

    #[test]
    fn notes_body_contains_looks_up() {
        let (dir, _id) = store_with_note("needle in a haystack");
        let result = run(&args(dir.path(), None, Some("needle"), None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["notes"][0]["body"], "needle in a haystack");
    }

    #[test]
    fn notes_conflicting_params_all_rejected() {
        let (dir, id) = store_with_note("x");
        let msg = "usage: pass exactly one of: id | body_contains | count | no params (full list)";
        let combos = [
            args(dir.path(), Some(&id), None, Some(true)),
            args(dir.path(), None, Some("x"), Some(true)),
            args(dir.path(), Some(&id), Some("x"), None),
        ];
        for combo in combos {
            let result = run(&combo);
            assert_eq!(result.is_error, Some(true), "should reject: {combo:?}");
            assert_eq!(text_of(&result), msg, "should name valid forms: {combo:?}");
        }
    }

    #[test]
    fn notes_without_store_is_store_absent() {
        let dir = tempfile::tempdir().unwrap();
        let result = run(&args(dir.path(), None, None, None));
        assert_ne!(
            result.is_error,
            Some(true),
            "store-absent must be a success: {result:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["store"], "absent");
    }
}
