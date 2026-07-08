//! `notes` — the loop's store browser: one note by id, a body-substring search,
//! or the whole inventory with each note's current anchor status.
//!
//! A pure read (invariant #8): it resolves notes against current code but never
//! writes. The three lookups are mutually exclusive — `id`, `body_contains`, and
//! `count` name different shapes — so a request that sets more than one is a
//! usage error before the store is even touched. A repo with no store answers
//! `{"store":"absent"}`, a success, so an ambient server does not error-spam
//! repositories that never adopted ynotes.

use std::path::Path;

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::{
    ListCountData, ListData, LookupData, ShowData, count_view, malformed_view, note_view,
};
use ynotes::{GitContext, IdMatch, ResolvedNote, SourceFile, Store, resolve};

use crate::tools::{discover_for, ok_json, store_absent, tool_err};

/// Arguments for the `notes` tool. Field doc comments become the JSON-schema
/// descriptions the client shows an agent.
///
/// The schema derive is pointed at rmcp's re-exported `schemars` (rather than a
/// direct dependency) so the crate cannot pick up a second, incompatible
/// schemars version — it always uses exactly the one rmcp compiled against.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct NotesArgs {
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
/// request that sets more than one is a usage error — checked before the store
/// is touched, so it does not depend on store state. Otherwise: `id` resolves a
/// single note against its stored target (as `show` does) into `showData`;
/// `body_contains` runs `lookup` into `lookupData`; neither runs `list` into
/// `listData`, or a `count` breakdown into `listCountData`. A repo with no store
/// answers `{"store":"absent"}` — a pure read never errors on adoption.
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

    let store = match discover_for(Path::new(".")) {
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
            warnings: Vec::new(),
        }),
        Ok(result) => ok_json(&ListData {
            notes: result.notes.iter().map(|n| note_view(n, false)).collect(),
            malformed: result.malformed.iter().map(malformed_view).collect(),
            warnings: Vec::new(),
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
            let n = candidates.len();
            let listed: Vec<String> = candidates
                .iter()
                .map(|m| m.id.chars().take(16).collect::<String>())
                .collect();
            return tool_err(
                "usage",
                format!(
                    "`{id}` is ambiguous — {n} candidates: {}",
                    listed.join(", ")
                ),
            );
        }
        Err(e) => return tool_err("engine", e),
    };

    let workdir = match store.workdir() {
        Ok(w) => w.to_path_buf(),
        Err(e) => return tool_err("engine", e),
    };
    let abs = workdir.join(&note.target);
    // Missing/unreadable target => empty source => the note orphans and is still
    // shown, exactly as `list`/`show` resolve a note.
    let source =
        SourceFile::read(&abs).unwrap_or_else(|_| SourceFile::from_lines(abs.clone(), Vec::new()));
    let git = GitContext::discover(&workdir);
    let resolution = resolve(&note.bundle, &source, git.as_ref());

    let mut warnings = Vec::new();
    if !abs.exists() {
        warnings.push(format!(
            "`{}` does not exist (the note's target file is gone)",
            note.target
        ));
    }

    let resolved = ResolvedNote {
        note,
        resolution,
        relocated_from: None,
    };
    ok_json(&ShowData {
        note: note_view(&resolved, false),
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

    fn args(id: Option<&str>, body_contains: Option<&str>, count: Option<bool>) -> NotesArgs {
        NotesArgs {
            id: id.map(ToOwned::to_owned),
            body_contains: body_contains.map(ToOwned::to_owned),
            count,
        }
    }

    #[test]
    fn notes_no_params_lists_all() {
        let (dir, _id) = store_with_note("first note");
        let _cwd = CwdGuard::enter(dir.path());
        let result = run(&args(None, None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["notes"][0]["body"], "first note");
    }

    #[test]
    fn notes_count_mode_reports_breakdown() {
        let (dir, _id) = store_with_note("x");
        let _cwd = CwdGuard::enter(dir.path());
        let result = run(&args(None, None, Some(true)));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["count"]["total"], 1);
        assert!(v.get("notes").is_none(), "count mode omits the notes array");
    }

    #[test]
    fn notes_by_id_shows_one_note() {
        let (dir, id) = store_with_note("the specific one");
        let _cwd = CwdGuard::enter(dir.path());
        let result = run(&args(Some(&id), None, None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["note"]["id"], id);
        assert_eq!(v["note"]["body"], "the specific one");
    }

    #[test]
    fn notes_body_contains_looks_up() {
        let (dir, _id) = store_with_note("needle in a haystack");
        let _cwd = CwdGuard::enter(dir.path());
        let result = run(&args(None, Some("needle"), None));
        assert_ne!(result.is_error, Some(true), "unexpected error: {result:?}");
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["notes"][0]["body"], "needle in a haystack");
    }

    #[test]
    fn notes_conflicting_params_all_rejected() {
        let (dir, id) = store_with_note("x");
        let _cwd = CwdGuard::enter(dir.path());
        let msg = "usage: pass exactly one of: id | body_contains | count | no params (full list)";
        let combos = [
            args(Some(&id), None, Some(true)),
            args(None, Some("x"), Some(true)),
            args(Some(&id), Some("x"), None),
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
        let _cwd = CwdGuard::enter(dir.path());
        let result = run(&args(None, None, None));
        assert_ne!(
            result.is_error,
            Some(true),
            "store-absent must be a success: {result:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&text_of(&result)).unwrap();
        assert_eq!(v["store"], "absent");
    }
}
