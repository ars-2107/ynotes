//! A note: a piece of context the user attached to a region of code, plus the
//! [`SelectorBundle`] that lets it re-anchor.
//!
//! The note id is content-addressed over `(target, scope, selector bundle,
//! body)` — and deliberately **not** the timestamp. Identical context
//! therefore always hashes to the same id, which makes `save` idempotent (an
//! agent retrying a tool call cannot duplicate) and ids deterministic across
//! machines (testable). Editing the body produces a *new* note rather than
//! mutating one in place; `Store::save_superseding` then retires the prior
//! note at that location, so a re-save replaces rather than accumulates.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::selector::{SelectorBundle, sha256_hex};

/// What a note is *about* — the user's intent, independent of where the anchor
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

/// A stored note.
///
/// Serialised as one JSON object per file under the store's `notes/`
/// directory. Both timestamps are mutable metadata, excluded from the id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Lowercase hex SHA-256 over `(target, scope, bundle, body)`.
    pub id: String,
    /// Store-root-relative, forward-slash path of the file this note targets.
    pub target: String,
    /// What the note is about.
    pub scope: Scope,
    /// The redundant location description used to re-anchor.
    pub bundle: SelectorBundle,
    /// The context text itself — the payload handed to an agent on a match.
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
    /// The id is a pure function of `(target, scope, bundle, body)` — **not**
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
        let id = compute_id(&target, scope, &bundle, &body)?;
        Ok(Self {
            id,
            target,
            scope,
            bundle,
            body,
            created_at: now,
            updated_at: now,
        })
    }

    /// Produces the note that supersedes this one with a refreshed `bundle`
    /// (used by `reanchor`). `created_at` and `body` are preserved — it is
    /// the *same* note re-anchored, not a new one — so `created_at` provenance
    /// survives; `updated_at` and the content-addressed `id` are recomputed.
    /// If the bundle is unchanged the id is identical, so this is idempotent.
    ///
    /// # Errors
    ///
    /// As [`Note::new`]: [`Error::Invalid`] if the identity cannot be hashed.
    pub fn with_bundle(self, bundle: SelectorBundle) -> Result<Self> {
        let id = compute_id(&self.target, self.scope, &bundle, &self.body)?;
        Ok(Self {
            id,
            bundle,
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
    /// final identity tuple — no intermediate id that would be invalidated by
    /// the next call.
    ///
    /// # Errors
    ///
    /// As [`Note::new`]: [`Error::Invalid`] if the identity cannot be hashed.
    pub fn with_bundle_and_body(self, bundle: SelectorBundle, body: String) -> Result<Self> {
        let id = compute_id(&self.target, self.scope, &bundle, &body)?;
        Ok(Self {
            id,
            bundle,
            body,
            updated_at: jiff::Timestamp::now(),
            ..self
        })
    }
}

/// The content-addressed id: SHA-256 over a canonical JSON encoding of
/// `(target, scope, bundle, body)`. Deterministic and timestamp-free, so
/// identical notes share an id (idempotent saves) and a test can assert it.
fn compute_id(target: &str, scope: Scope, bundle: &SelectorBundle, body: &str) -> Result<String> {
    let identity = (target, scope, bundle, body);
    let bytes = serde_json::to_vec(&identity)
        .map_err(|e| Error::Invalid(format!("cannot serialise note identity for hashing: {e}")))?;
    Ok(sha256_hex(&bytes))
}
