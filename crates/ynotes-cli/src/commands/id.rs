//! Shared note-id parsing for the commands that select a note by id or by
//! hex prefix (`delete`, `update`).
//!
//! One module owns [`MIN_PREFIX`] and [`validate_prefix`] so the two
//! subcommands cannot drift on what counts as a well-formed id.

use ynotes::{Note, Store};

use crate::command_error::CommandError;

/// Smallest accepted prefix. Four hex characters distinguish 65,536 ids —
/// enough headroom for ergonomic short forms while keeping a stray single
/// character (`yn delete a`) from accidentally matching anything.
pub(super) const MIN_PREFIX: usize = 4;

/// Reject malformed prefixes (non-hex or shorter than [`MIN_PREFIX`]) up
/// front, so a user mistake at the call site surfaces as a usage error
/// (exit `2`) rather than a runtime "not found".
///
/// # Errors
///
/// Returns [`CommandError::Usage`] if `id` is shorter than [`MIN_PREFIX`] or
/// contains a non-hex character.
pub(super) fn validate_prefix(id: &str) -> Result<(), CommandError> {
    if id.len() < MIN_PREFIX {
        return Err(CommandError::Usage(format!(
            "`{id}` is too short: id prefixes must be at least {MIN_PREFIX} hex characters"
        )));
    }
    if !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CommandError::Usage(format!(
            "`{id}` is not a valid note id (expected lowercase hex)"
        )));
    }
    Ok(())
}

/// Resolve `id` (or hex prefix) to exactly one note. Multiple matches and no
/// match are both surfaced as user-input failures (exit `2`) — the user gave
/// us a selector that did not pick out a single note, which is a call-site
/// mistake to fix. Shared by `update` and `show` so the two cannot drift on
/// how a prefix is resolved or how ambiguity is reported.
///
/// # Errors
///
/// [`CommandError::Usage`] when the prefix matches no note or more than one;
/// [`CommandError::Engine`] if the store index or a note record cannot be read.
pub(super) fn resolve_unique(store: &Store, id: &str) -> Result<Note, CommandError> {
    let matches = store.find_by_id_prefix(id)?;
    match matches.len() {
        0 => Err(CommandError::Usage(format!(
            "no note matches `{id}` (use `yn list` to see available ids)"
        ))),
        1 => Ok(matches.into_iter().next().expect("len == 1")),
        n => {
            let candidates: Vec<String> = matches
                .iter()
                .map(|m| m.id.chars().take(16).collect::<String>())
                .collect();
            Err(CommandError::Usage(format!(
                "`{id}` is ambiguous — {n} candidates: {}",
                candidates.join(", ")
            )))
        }
    }
}
