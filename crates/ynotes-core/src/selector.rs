//! Capture and serialise redundant selectors for a code region.
//!
//! Every bundle includes exact text and the saved position. Git transport and
//! syntax-tree selectors are optional. [`crate::anchor`] resolves the bundle
//! against current code and reports whether the region is intact or missing.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::source::{LineRange, SourceFile};

/// On-disk schema version of a [`SelectorBundle`].
///
/// Bumped whenever the bundle layout changes so the resolver can migrate (or
/// refuse) an older record explicitly instead of misreading it.
pub const SCHEMA: u32 = 1;

/// Lines of context captured on each side of the region for the quote
/// selector. Three is enough to disambiguate a non-unique snippet without
/// being so wide that an unrelated edit nearby looks like drift.
const CONTEXT_LINES: u32 = 3;

/// The region's text plus the lines bracketing it.
///
/// The exact text is the language-agnostic floor: if every other rung fails,
/// this is what lets a human (or agent) see the lost context and re-place it,
/// honouring the never-silently-dropped invariant. `prefix`/`suffix` make an
/// otherwise duplicated snippet uniquely locatable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteSelector {
    /// Verbatim text of the region at save time (lines joined with `\n`).
    pub exact: String,
    /// Up to [`CONTEXT_LINES`] lines immediately before the region.
    pub prefix: String,
    /// Up to [`CONTEXT_LINES`] lines immediately after the region.
    pub suffix: String,
    /// Lowercase hex SHA-256 of `exact`, for an O(1) "did this region survive
    /// verbatim?" check before any search.
    pub sha256: String,
    /// How many times `exact` occurred in the file when the selector was
    /// captured. A moved match that was unique then and remains unique now is
    /// positive identity evidence even when reordering changed both neighbours.
    /// `None` only for schema-v1 records, whose resolver stays conservative
    /// unless another saved rung can reconstruct or corroborate the identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrences_at_capture: Option<u32>,
}

/// The git anchor: the commit, the repo-relative path, and the region's range
/// *in that commit's coordinates*, so R1 can transport it forward through
/// later commits and work-tree edits. Absent when the file was not tracked at
/// save time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSelector {
    /// `HEAD` commit id at save time.
    pub commit: String,
    /// Work-tree-relative path (forward slashes) at save time.
    pub path: String,
    /// The region's line range in `commit`'s blob, the baseline R1
    /// transports forward.
    ///
    /// Captured in the *commit's* coordinates, not the working tree's, so the
    /// `(commit, range)` pair stays internally consistent even when `save` or
    /// `reanchor` ran against a dirty tree. `None` only for a note written
    /// before this field existed; R1 then falls back to the position
    /// selector's range.
    #[serde(default)]
    pub range: Option<LineRange>,
}

/// Where the region sat at save time. The weakest signal, a tie-breaker when
/// stronger rungs disagree, never authoritative on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionSelector {
    /// The original line range.
    pub range: LineRange,
    /// The file's line count at save time, so a later resolver can tell a
    /// truncated file from a moved region.
    pub file_lines: u32,
}

/// The complete, redundant location description stored with every note.
///
/// Resolution combines independent selectors. Git transport and syntax-tree
/// selectors are optional when their evidence is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectorBundle {
    /// Schema version this bundle was written under. See [`SCHEMA`].
    pub schema: u32,
    /// The always-present text floor.
    pub quote: QuoteSelector,
    /// The original position (tie-breaker).
    pub position: PositionSelector,
    /// The git anchor, if the file was tracked when the note was saved.
    #[serde(default)]
    pub git: Option<GitSelector>,
    /// The structural (tree-sitter) anchor, if the language is parseable.
    #[serde(default)]
    pub structural: Option<crate::structural::StructuralSelector>,
}

impl SelectorBundle {
    /// Captures a bundle describing `range` within `file`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Invalid`] if `range` extends past the end of
    /// `file` (propagated from [`SourceFile::text`]).
    pub fn capture(file: &SourceFile, range: LineRange) -> Result<Self> {
        let exact = file.text(range)?;
        let (prefix, suffix) = file.surrounding(range, CONTEXT_LINES)?;
        let needle: Vec<&str> = exact.split('\n').collect();
        let occurrences = file
            .lines_slice()
            .windows(needle.len())
            .filter(|window| window.iter().zip(&needle).all(|(line, part)| line == part))
            .count();
        Ok(Self {
            schema: SCHEMA,
            quote: QuoteSelector {
                sha256: sha256_hex(exact.as_bytes()),
                exact,
                prefix,
                suffix,
                // The selected region itself is one occurrence, so conversion
                // cannot realistically overflow. Saturate defensively rather
                // than making selector capture fallible for file size alone.
                occurrences_at_capture: Some(u32::try_from(occurrences).unwrap_or(u32::MAX)),
            },
            position: PositionSelector {
                range,
                file_lines: file.line_count(),
            },
            git: None,
            structural: None,
        })
    }

    /// Attaches a git anchor to a captured bundle (builder style).
    #[must_use]
    pub fn with_git(mut self, git: GitSelector) -> Self {
        self.git = Some(git);
        self
    }

    /// Attaches a structural anchor to a captured bundle (builder style).
    #[must_use]
    pub fn with_structural(mut self, structural: crate::structural::StructuralSelector) -> Self {
        self.structural = Some(structural);
        self
    }

    /// Captures the *complete* bundle for `range` of `file`: the text and
    /// position floor plus, when available, the git and tree-sitter anchors.
    ///
    /// This is the one place bundle construction lives, so `save` and
    /// `reanchor` cannot drift apart (and no anchoring logic leaks into a
    /// binary-only module). Git/structural are best-effort: their absence is
    /// normal, never an error.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Invalid`] if `range` extends past the end of
    /// `file` (propagated from [`SelectorBundle::capture`]).
    pub fn capture_full(file: &SourceFile, range: LineRange) -> Result<Self> {
        let mut bundle = Self::capture(file, range)?;

        let dir = file
            .path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        if let Some(ctx) = crate::git::GitContext::discover(dir)
            && let (Some(commit), Some(path)) = (ctx.head_commit(), ctx.relativize(file.path()))
        {
            // Only anchor to git when the path is actually a blob in HEAD.
            // For an untracked file the git rung's baseline is empty,
            // `git diff HEAD -- <path>` shows nothing, so transport would
            // report a spurious result; leaving `git: None` makes R1
            // honestly `Skipped` and the other rungs carry the resolution.
            if ctx.tracks(&commit, &path) {
                // Translate the working-tree `range` into `commit`'s own
                // coordinates: that, not `position.range`, which
                // `reanchor` rewrites, is the baseline R1 transports, so
                // the `(commit, range)` pair stays consistent on a dirty
                // tree.
                let git_range = ctx.range_in_commit(&commit, &path, range);
                bundle = bundle.with_git(GitSelector {
                    commit,
                    path,
                    range: Some(git_range),
                });
            }
        }

        if let Some(lang) = crate::structural::Language::from_path(file.path())
            && let Some(sel) = crate::structural::capture(&file.text_all(), lang, range)
        {
            bundle = bundle.with_structural(sel);
        }

        Ok(bundle)
    }
}

/// Lowercase hex SHA-256 of `bytes`.
///
/// A free function rather than pulling in a hex crate: the engine only ever
/// needs this one direction (bytes to hex), so the dependency is not worth it.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        // Writing to a `String` is infallible; the `Result` is discarded
        // deliberately rather than `unwrap`ped to keep this panic-free.
        let _ = write!(out, "{byte:02x}");
    }
    out
}
