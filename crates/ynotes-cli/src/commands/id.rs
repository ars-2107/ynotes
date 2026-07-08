//! Shared note-id selection for the commands that pick a note by id or by
//! hex prefix (`delete`, `update`, `show`).
//!
//! The policy itself — what counts as a well-formed prefix, and the
//! One/Ambiguous/None resolution — is engine-owned
//! ([`ynotes::validate_id_prefix`] and [`ynotes::Store::match_id_prefix`]);
//! this module is the presentational thread onto it, giving the subcommands
//! one place that maps the engine's answers to the CLI's usage messages so
//! they cannot drift on wording.

use ynotes::{IdMatch, Note, Store};

use crate::command_error::CommandError;

/// Reject malformed prefixes (non-hex or too short) up front, so a user
/// mistake at the call site surfaces as a usage error (exit `2`) rather than
/// a runtime "not found".
///
/// Thin presentation over [`ynotes::validate_id_prefix`]: the engine owns the
/// policy, and the classification in [`crate::command_error`] maps its
/// [`ynotes::Error::InvalidIdPrefix`] to the usage exit code.
///
/// # Errors
///
/// Returns [`CommandError::Usage`] if `id` is shorter than the engine's
/// minimum prefix length or contains a non-hex character.
pub(super) fn validate_prefix(id: &str) -> Result<(), CommandError> {
    ynotes::validate_id_prefix(id).map_err(CommandError::from)
}

/// Resolve `id` (or hex prefix) to exactly one note. Multiple matches and no
/// match are both surfaced as user-input failures (exit `2`) — the user gave
/// us a selector that did not pick out a single note, which is a call-site
/// mistake to fix. Shared by `update` and `show` so the two cannot drift on
/// how a prefix is resolved or how ambiguity is reported.
///
/// Presentation over [`ynotes::Store::match_id_prefix`]: the engine returns the
/// One/Ambiguous/None outcome as data, and this turns the two non-unique cases
/// into the CLI's usage messages.
///
/// # Errors
///
/// [`CommandError::Usage`] when the prefix matches no note or more than one;
/// [`CommandError::Engine`] if the store index or a note record cannot be read.
pub(super) fn resolve_unique(store: &Store, id: &str) -> Result<Note, CommandError> {
    match store.match_id_prefix(id)? {
        IdMatch::One(note) => Ok(note),
        IdMatch::None => Err(CommandError::Usage(format!(
            "no note matches `{id}` (use `yn list` to see available ids)"
        ))),
        IdMatch::Ambiguous(candidates) => {
            let n = candidates.len();
            let listed: Vec<String> = candidates
                .iter()
                .map(|m| m.id.chars().take(16).collect::<String>())
                .collect();
            Err(CommandError::Usage(format!(
                "`{id}` is ambiguous — {n} candidates: {}",
                listed.join(", ")
            )))
        }
    }
}
