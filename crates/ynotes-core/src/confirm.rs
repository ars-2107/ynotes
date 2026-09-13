//! Explicit review confirmation for stale notes.
//!
//! Re-anchoring refreshes only location selectors and deliberately preserves a
//! note's review basis. This module owns the inverse operation: after a human
//! or agent has checked the note body against current code, [`confirm`]
//! advances the review basis and re-captures the selectors in one auditable
//! write.
//!
//! Trust must never be established against a guessed line range, so two
//! resolutions are refused: an orphan, which has no range at all, and a region
//! located only by *similarity* (fuzzy, or an ambiguous quote) rather than
//! identity. Both refusals carry their remedy, the second is `reanchor`,
//! which refreshes the selectors so the following `confirm` resolves exactly.

use std::path::Path;

use crate::anchor::resolve;
use crate::error::{Error, Result};
use crate::git::GitContext;
use crate::note::Note;
use crate::selector::SelectorBundle;
use crate::source::{LineRange, SourceFile};
use crate::store::Store;

/// The result of explicitly confirming one note against current code.
#[derive(Debug, Clone)]
pub struct ConfirmResult {
    /// The confirmed note, carrying its current content-addressed id.
    pub note: Note,
    /// The id this confirmation superseded.
    pub previous_id: String,
    /// The current region against which the note was reviewed.
    pub range: LineRange,
    /// Whether the content-addressed identity changed.
    pub changed: bool,
    /// Whether a new record was created in the store.
    pub created: bool,
}

/// Confirm `original`'s body against its currently resolved code region.
///
/// This is the only body-preserving operation that advances the persistent
/// review basis. The anchor ladder must locate the region first; no saved-line
/// fallback is allowed. The replacement record is written before the old one
/// is retired so an interrupted operation cannot lose the note.
///
/// # Errors
///
/// Returns [`Error::Unconfirmable`] when the note is orphaned,
/// [`Error::Unidentified`] when only a similarity rung located its region,
/// [`Error::SymlinkEscape`] when its target was replaced by an escaping
/// symlink, or propagates store, source-read, selector-capture, and write
/// failures.
pub fn confirm(store: &Store, original: Note) -> Result<ConfirmResult> {
    let workdir = store.workdir()?.to_path_buf();
    let abs = workdir.join(&original.target);
    store.reject_symlink_escape(&abs)?;
    let source = SourceFile::read(&abs)?;
    let git = GitContext::discover(abs.parent().unwrap_or_else(|| Path::new(".")));
    let resolution = resolve(&original.bundle, &source, git.as_ref());
    let range = resolution.range.ok_or_else(|| Error::Unconfirmable {
        id: original.id.clone(),
    })?;
    // A range alone is not enough: the fuzzy rung returns one for any
    // sufficiently similar region, and capturing the basis there would record
    // the body as reviewed against code nobody read. `reanchor` refreshes the
    // selectors first, which is precisely what makes the documented
    // reanchor -> confirm path resolve exactly.
    if !resolution.is_identified() {
        return Err(Error::Unidentified {
            id: original.id.clone(),
        });
    }

    let bundle = SelectorBundle::capture_full(&source, range)?;
    let confirmed = original.clone().confirmed(bundle)?;
    let changed = confirmed.id != original.id;
    let created = if changed {
        let created = store.save(&confirmed)?;
        store.remove(&original)?;
        created
    } else {
        false
    };

    Ok(ConfirmResult {
        previous_id: original.id,
        note: confirmed,
        range,
        changed,
        created,
    })
}
