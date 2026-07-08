//! Pre-engine safety check shared by the write-path commands.
//!
//! The guard itself lives in the engine as [`ynotes::Store::reject_symlink_escape`]
//! — it is behaviour both front-ends need, so it belongs in the library. This
//! binary-only helper is the presentational thread onto it: it maps the engine's
//! typed [`ynotes::Error::SymlinkEscape`] to the CLI's usage exit code and gives
//! the two write-path call sites (`save`, `update`) one uniform name to call.

use std::path::Path;

use ynotes::Store;

use crate::command_error::CommandError;

/// Refuse a *symlink* whose canonical target lives outside the work tree of
/// `store`, as a [`CommandError`].
///
/// Thin presentation over [`ynotes::Store::reject_symlink_escape`]: the escape
/// surfaces as [`CommandError::Usage`] (exit `2`) via the engine-error
/// classification in [`crate::command_error`], everything else as an engine
/// failure. See the engine method for why the check is narrowed to genuine
/// symlinks.
///
/// # Errors
///
/// [`CommandError::Usage`] if `file` is a symlink resolving outside the work
/// tree; [`CommandError::Engine`] if the store has no work tree.
pub(crate) fn reject_if_escapes_workdir(file: &Path, store: &Store) -> Result<(), CommandError> {
    store
        .reject_symlink_escape(file)
        .map_err(CommandError::from)
}
