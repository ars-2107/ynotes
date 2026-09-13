//! `confirm` MCP tool: explicit review-basis advancement for one note.

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::confirm_view;
use ynotes::{Error, IdMatch, confirm};

use crate::tools::{discover_for, explicit_root, ok_json, tool_err};

/// Arguments for the `confirm` tool.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct ConfirmArgs {
    /// Absolute path to the repository root this request addresses.
    pub(crate) root: String,
    /// Full note id or an unambiguous hexadecimal prefix of at least 4 chars.
    pub(crate) id: String,
}

/// Confirm the selected note against its currently resolved code.
pub(crate) fn run(args: &ConfirmArgs) -> CallToolResult {
    if let Err(e) = ynotes::validate_id_prefix(&args.id) {
        return tool_err("usage", e);
    }
    let root = match explicit_root(&args.root) {
        Ok(root) => root,
        Err(message) => return tool_err("usage", message),
    };
    let store = match discover_for(&root) {
        Ok(Some(store)) => store,
        Ok(None) => return tool_err("usage", "no ynotes store found (save a note first)"),
        Err(e) => return tool_err("engine", e),
    };
    let note = match store.match_id_prefix(&args.id) {
        Ok(IdMatch::One(note)) => note,
        Ok(IdMatch::None) => {
            // Ids are content-addressed, so they rotate on reanchor, update and
            // confirm. A caller reusing one from earlier in the session is the
            // likeliest way to land here, and a bare "no match" reads as "the
            // note is gone" when it has merely been renumbered.
            return tool_err(
                "usage",
                format!(
                    "no note matches `{}`; ids rotate when a note is re-anchored, \
                     updated or confirmed, so re-resolve it with the notes tool \
                     (by target, or by a substring of its body) rather than \
                     reusing a remembered id",
                    args.id
                ),
            );
        }
        Ok(IdMatch::Ambiguous(candidates)) => {
            return tool_err("usage", ynotes::ambiguous_id_message(&args.id, &candidates));
        }
        Err(e) => return tool_err("engine", e),
    };

    match confirm(&store, note) {
        Ok(result) => ok_json(&confirm_view(&result)),
        Err(
            e @ (Error::Unconfirmable { .. }
            | Error::Unidentified { .. }
            | Error::SymlinkEscape { .. }),
        ) => tool_err("usage", e),
        Err(e) => tool_err("engine", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ynotes::{LineRange, Note, Scope, SelectorBundle, SourceFile, Store};

    fn text_of(r: &CallToolResult) -> String {
        r.content[0]
            .as_text()
            .expect("text content block")
            .text
            .clone()
    }

    /// Save a note on `file` covering its first three lines.
    fn seed(root: &std::path::Path) -> String {
        let store = Store::init(root).unwrap();
        let file = root.join("x.rs");
        std::fs::write(&file, "fn target() {\n    unique_marker();\n}\n").unwrap();
        let src = SourceFile::read(&file).unwrap();
        let bundle = SelectorBundle::capture_full(&src, LineRange::new(1, 3).unwrap()).unwrap();
        let note = Note::new(
            "x.rs".to_owned(),
            Scope::Range,
            bundle,
            "why this is so".to_owned(),
        )
        .unwrap();
        store.save(&note).unwrap();
        note.id
    }

    /// The MCP face must refuse the same thing the CLI does. `confirm` records
    /// that a body was checked against a region, so an orphan, which has no
    /// region, cannot be confirmed. Classified `usage`, not `engine`: the
    /// caller asked for something incoherent, and can act on that.
    #[test]
    fn confirm_refuses_an_orphan_as_a_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let id = seed(dir.path());
        // Obliterate the anchored content so no content rung can locate it.
        std::fs::write(dir.path().join("x.rs"), "fn something_else() {}\n").unwrap();

        let result = run(&ConfirmArgs {
            root: dir.path().display().to_string(),
            id: id.clone(),
        });
        assert_eq!(result.is_error, Some(true));
        let text = text_of(&result).to_lowercase();
        assert!(
            text.contains("orphan"),
            "the refusal must say why, got: {text}"
        );

        // Refused, not silently swallowed: the note survives (invariant #4).
        let store = Store::open_at_root(dir.path()).unwrap().unwrap();
        assert_eq!(store.notes_for("x.rs").unwrap().notes[0].id, id);
    }

    /// Invariant #13: a request names its repository, and store selection is
    /// exact to it. A child directory that never adopted ynotes must not quietly
    /// resolve against an ancestor's store, that would write a note into, or
    /// read one out of, a repository the caller did not name.
    #[test]
    fn store_selection_never_walks_up_to_a_parent_store() {
        let parent = tempfile::tempdir().unwrap();
        let id = seed(parent.path());

        let child = parent.path().join("nested");
        std::fs::create_dir(&child).unwrap();

        let result = run(&ConfirmArgs {
            root: child.display().to_string(),
            id,
        });
        assert_eq!(
            result.is_error,
            Some(true),
            "the child has no store of its own, so there is nothing to confirm"
        );
        let text = text_of(&result).to_lowercase();
        assert!(
            text.contains("no ynotes store"),
            "the error must name the real problem, no store here, rather \
             than reporting the parent's notes, got: {text}"
        );
    }
}
