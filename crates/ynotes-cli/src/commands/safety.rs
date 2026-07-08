//! Pre-engine safety checks shared by the write-path commands.
//!
//! [`reject_if_escapes_workdir`] is the save/update mirror of `reanchor`'s
//! in-engine canonicalisation guard ([`crate`](crate)'s engine sits in the
//! library — see `src/reanchor.rs`'s symlink-escape branch). The lexical
//! [`ynotes::Store::relativize`] catches a path *spelled* outside the work
//! tree, but it deliberately works for files that do not yet exist and so
//! cannot follow symlinks: a path whose name lives inside the store but
//! whose symlink target points elsewhere therefore passes it. The check here
//! closes that gap — and *only* that gap — by canonicalising both sides when
//! the path is a symlink. Non-symlink paths (regular files, directories,
//! devices like `/dev/null`) are left to the lexical check so the user sees
//! one consistent "outside the ynotes store" message for that condition,
//! never a misleading "symlink target escapes" parenthetical for a path
//! that has no symlink in it.

use std::path::Path;

use ynotes::Store;

use crate::command_error::CommandError;

/// Refuse a *symlink* whose canonical target lives outside the work tree of
/// `store`.
///
/// Scope is deliberately narrow: a non-symlink path falls through here even
/// if it sits outside the work tree, because the lexical
/// [`ynotes::Store::relativize`] is the authoritative outside-the-store
/// check for ordinary paths — it produces a single, accurate
/// [`ynotes::Error::OutsideStore`] for every non-symlink case (absolute
/// outside paths, devices, paths that do not exist yet) and the front-end
/// already maps it to the usage exit code (`2`).
///
/// A no-op when `file` does not exist on disk (no metadata to read): the
/// calling command then fails further down with a clearer missing-file
/// error.
pub(crate) fn reject_if_escapes_workdir(file: &Path, store: &Store) -> Result<(), CommandError> {
    // `symlink_metadata`, not `metadata`: we want the link itself, not what
    // it points at. A missing file is silently ignored — the lexical check
    // and the subsequent read will produce the right diagnostic.
    let Ok(meta) = file.symlink_metadata() else {
        return Ok(());
    };
    if !meta.file_type().is_symlink() {
        return Ok(());
    }

    let workdir = store.workdir().map_err(CommandError::Engine)?;
    let Ok(canon_file) = file.canonicalize() else {
        return Ok(());
    };
    let Ok(canon_work) = workdir.canonicalize() else {
        return Ok(());
    };
    if canon_file.strip_prefix(&canon_work).is_err() {
        return Err(CommandError::Usage(format!(
            "`{}` is a symlink whose target escapes the work tree at `{}` (resolves to `{}`)",
            file.display(),
            canon_work.display(),
            canon_file.display(),
        )));
    }
    Ok(())
}
