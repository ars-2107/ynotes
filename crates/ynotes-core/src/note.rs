//! Stored note content, selectors, and review state.
//!
//! IDs hash `(target, scope, selector bundle, review basis, body)`, excluding
//! timestamps. Identical saves are idempotent while these inputs stay unchanged.
//! Updates create a new record and retire the previous one.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::selector::{SelectorBundle, sha256_hex};

/// What a note is *about*, the user's intent, independent of where the anchor
/// currently resolves.
///
/// Resolution treats the three scopes differently: a `File` note matches any
/// query on its file; `Range`/`Line` notes match by overlap with the queried
/// interval. Stored explicitly because it cannot be recovered from the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Attached to a single line.
    Line,
    /// Attached to a contiguous range of lines.
    Range,
    /// Attached to the file as a whole.
    File,
}

/// The code content against which the note was last explicitly reviewed.
///
/// Selector refreshes preserve this value. Consequently, moving or
/// re-capturing an anchor cannot silently turn stale prose back into trusted
/// context. Only a save, update, or explicit confirmation establishes a new
/// basis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewBasis {
    /// SHA-256 of the reviewed region's exact text.
    pub sha256: String,
}

impl ReviewBasis {
    fn from_bundle(bundle: &SelectorBundle) -> Self {
        Self {
            sha256: bundle.quote.sha256.clone(),
        }
    }
}

/// On-disk shape of a [`Note`], tolerating records written before the review
/// lifecycle existed.
///
/// If `review_basis` is absent, use the bundle's quote hash, the code captured
/// when the note was saved. This keeps development stores readable without
/// treating their contents as reviewed against newer code.
#[derive(Deserialize)]
struct NoteRepr {
    id: String,
    target: String,
    scope: Scope,
    bundle: SelectorBundle,
    #[serde(default)]
    review_basis: Option<ReviewBasis>,
    body: String,
    created_at: jiff::Timestamp,
    updated_at: jiff::Timestamp,
}

impl From<NoteRepr> for Note {
    fn from(repr: NoteRepr) -> Self {
        let review_basis = repr
            .review_basis
            .unwrap_or_else(|| ReviewBasis::from_bundle(&repr.bundle));
        Self {
            id: repr.id,
            target: repr.target,
            scope: repr.scope,
            bundle: repr.bundle,
            review_basis,
            body: repr.body,
            created_at: repr.created_at,
            updated_at: repr.updated_at,
        }
    }
}

/// A stored note.
///
/// Serialised as one JSON object per file under the store's `notes/`
/// directory. Both timestamps are mutable metadata, excluded from the id.
///
/// A record written before `review_basis` existed still loads: the basis is
/// seeded from the bundle's quote hash, reproducing the pre-upgrade semantics
/// rather than rejecting the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "NoteRepr")]
pub struct Note {
    /// Lowercase hex SHA-256 over `(target, scope, bundle, review basis, body)`.
    pub id: String,
    /// Store-root-relative, forward-slash path of the file this note targets.
    pub target: String,
    /// What the note is about.
    pub scope: Scope,
    /// The redundant location description used to re-anchor.
    pub bundle: SelectorBundle,
    /// Code content against which the note was last explicitly reviewed.
    pub review_basis: ReviewBasis,
    /// The context text itself, the payload handed to an agent on a match.
    pub body: String,
    /// When the note was first created (metadata, not part of the id).
    pub created_at: jiff::Timestamp,
    /// When the note's mutable fields last changed.
    pub updated_at: jiff::Timestamp,
}

impl Note {
    /// Creates a note, stamping the timestamps to now and computing the
    /// content-addressed [`id`](Note::id).
    ///
    /// The id is a pure function of `(target, scope, bundle, review basis,
    /// body)`, **not**
    /// the timestamp. Two saves of identical context therefore produce the
    /// same id, so an agent that retries a `save` tool call cannot create a
    /// duplicate, and ids are deterministic across machines (testable). The
    /// timestamps are metadata only. Changing the body yields a *new* note
    /// (supersede semantics, like a content-addressed object store) rather
    /// than mutating one in place.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if the identity cannot be serialised for
    /// hashing (not expected for well-formed inputs; surfaced rather than
    /// panicked so a caller never loses a note to an `unwrap`).
    pub fn new(target: String, scope: Scope, bundle: SelectorBundle, body: String) -> Result<Self> {
        let now = jiff::Timestamp::now();
        let review_basis = ReviewBasis::from_bundle(&bundle);
        let id = compute_id(&target, scope, &bundle, &review_basis, &body)?;
        Ok(Self {
            id,
            target,
            scope,
            bundle,
            review_basis,
            body,
            created_at: now,
            updated_at: now,
        })
    }

    /// Produces the note that supersedes this one with a refreshed `bundle`
    /// (used by `reanchor`). `created_at` and `body` are preserved, it is
    /// the *same* note re-anchored, not a new one, so `created_at` provenance
    /// survives; `updated_at` and the content-addressed `id` are recomputed.
    /// If the bundle is unchanged the id is identical, so this is idempotent.
    ///
    /// # Errors
    ///
    /// As [`Note::new`]: [`Error::Invalid`] if the identity cannot be hashed.
    pub fn with_bundle(self, bundle: SelectorBundle) -> Result<Self> {
        let review_basis = self.review_basis.clone();
        let id = compute_id(&self.target, self.scope, &bundle, &review_basis, &self.body)?;
        Ok(Self {
            id,
            bundle,
            review_basis,
            updated_at: jiff::Timestamp::now(),
            ..self
        })
    }

    /// Produces the note that supersedes this one at a new `target`, with a
    /// bundle re-captured against the file at that new path (used by `reanchor`
    /// when it follows a file rename). `scope`, `body`, and `created_at` are
    /// preserved, it is the *same* note, moved to where its code now lives,
    /// so provenance survives the relocation; `updated_at` and the
    /// content-addressed `id` are recomputed over the new `(target, bundle)`.
    ///
    /// The id necessarily changes (the target is part of the identity, invariant
    /// #6), so a relocation is a new immutable record retiring the old one,
    /// exactly as an in-place re-anchor is.
    ///
    /// # Errors
    ///
    /// As [`Note::new`]: [`Error::Invalid`] if the identity cannot be hashed.
    pub fn relocated(self, target: String, bundle: SelectorBundle) -> Result<Self> {
        let review_basis = self.review_basis.clone();
        let id = compute_id(&target, self.scope, &bundle, &review_basis, &self.body)?;
        Ok(Self {
            id,
            target,
            bundle,
            review_basis,
            updated_at: jiff::Timestamp::now(),
            ..self
        })
    }

    /// Produces the note that supersedes this one with a refreshed `bundle`
    /// *and* a replaced `body` (used by `update`). `created_at` is preserved
    /// so the note's provenance survives the edit; `updated_at` and the
    /// content-addressed `id` are recomputed.
    ///
    /// Combined into one method rather than chaining `with_bundle` and a
    /// hypothetical `with_body` so the id is computed exactly once over the
    /// final identity tuple, no intermediate id that would be invalidated by
    /// the next call.
    ///
    /// # Errors
    ///
    /// As [`Note::new`]: [`Error::Invalid`] if the identity cannot be hashed.
    pub fn with_bundle_and_body(self, bundle: SelectorBundle, body: String) -> Result<Self> {
        let review_basis = ReviewBasis::from_bundle(&bundle);
        let id = compute_id(&self.target, self.scope, &bundle, &review_basis, &body)?;
        Ok(Self {
            id,
            bundle,
            review_basis,
            body,
            updated_at: jiff::Timestamp::now(),
            ..self
        })
    }

    /// Produces the note that supersedes this one after a reviewer has
    /// confirmed its body against `bundle`'s current code.
    ///
    /// Unlike [`Note::with_bundle`], this deliberately advances the review
    /// basis. The body and `created_at` provenance remain unchanged while the
    /// content-addressed id and `updated_at` are recomputed.
    ///
    /// # Errors
    ///
    /// As [`Note::new`]: [`Error::Invalid`] if the identity cannot be hashed.
    pub fn confirmed(self, bundle: SelectorBundle) -> Result<Self> {
        let review_basis = ReviewBasis::from_bundle(&bundle);
        let id = compute_id(&self.target, self.scope, &bundle, &review_basis, &self.body)?;
        Ok(Self {
            id,
            bundle,
            review_basis,
            updated_at: jiff::Timestamp::now(),
            ..self
        })
    }

    /// Whether `text` is identical to the code against which this note was
    /// last explicitly reviewed.
    ///
    #[must_use]
    pub fn review_matches(&self, text: &str) -> bool {
        self.review_basis.sha256 == sha256_hex(text.as_bytes())
    }
}

/// The content-addressed id: SHA-256 over a canonical JSON encoding of
/// `(target, scope, bundle, review basis, body)`. Deterministic and timestamp-free, so
/// identical notes share an id (idempotent saves) and a test can assert it.
fn compute_id(
    target: &str,
    scope: Scope,
    bundle: &SelectorBundle,
    review_basis: &ReviewBasis,
    body: &str,
) -> Result<String> {
    let identity = (target, scope, bundle, review_basis, body);
    let bytes = serde_json::to_vec(&identity)
        .map_err(|e| Error::Invalid(format!("cannot serialise note identity for hashing: {e}")))?;
    Ok(sha256_hex(&bytes))
}
