//! Shared note-id parsing for the commands that select a note by id or by
//! hex prefix (`delete`, `update`).
//!
//! One module owns [`MIN_PREFIX`] and [`validate_prefix`] so the two
//! subcommands cannot drift on what counts as a well-formed id.

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
