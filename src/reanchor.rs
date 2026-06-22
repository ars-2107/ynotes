//! `reanchor`: the deliberate, auditable maintenance pass that refreshes a
//! note's selectors to the code as it is now.
//!
//! Why this is a command and not something `query` does: a read must never
//! write (read-only mounts, concurrent readers, a future read-only MCP
//! server). Re-baking a location is also effectively irreversible — it
//! discards the original selectors. The safety rule, which is the whole
//! reason this is conservative: an orphaned note — one no content rung could
//! locate — is never re-anchored. It is left exactly as it is and reported
//! for a human; welding a note to maybe-wrong code is precisely the silent
//! failure invariant #4 forbids.

use crate::anchor::resolve;
use crate::error::{Error, Result};
use crate::git::GitContext;
use crate::selector::SelectorBundle;
use crate::source::{LineRange, SourceFile};
use crate::store::Store;

/// A note that was (or, in a dry run, would be) re-anchored.
#[derive(Debug, Clone)]
pub struct ReanchorChange {
    /// The note's target path.
    pub target: String,
    /// Id before re-anchoring.
    pub old_id: String,
    /// Id after (content-addressed over the refreshed bundle).
    pub new_id: String,
    /// Where the note used to be anchored.
    pub from: LineRange,
    /// Where it is now.
    pub to: LineRange,
}

/// Why a note was deliberately left untouched. A closed enum, deliberately —
/// the `--json` agent contract exposes this verbatim as `skipped[].reason_code`,
/// so consumers can branch on a stable code without parsing prose (invariant
/// #7). Adding a variant is a contract change; renaming one is a breaking
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReanchorSkipReason {
    /// No content rung located the region — re-anchoring would weld it to
    /// guesswork (invariant #4 / #8).
    Orphaned,
    /// The target file is not on disk.
    MissingTarget,
    /// The target's canonical path lies outside the work tree (a symlink
    /// swap after save, the post-save half of the symlink-escape guard).
    SymlinkEscape,
}

impl ReanchorSkipReason {
    /// The stable, lower-kebab-case classification the `--json` agent contract
    /// exposes as `skipped[].reason_code`. Consumers should branch on this,
    /// not on [`Self::human`].
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Orphaned => "orphaned",
            Self::MissingTarget => "missing-target",
            Self::SymlinkEscape => "symlink-escape",
        }
    }

    /// The free-form, user-facing explanation. Free to evolve for clarity —
    /// the stable code lives in [`Self::code`], so reworking the prose does
    /// not break the agent contract.
    #[must_use]
    pub fn human(self) -> &'static str {
        match self {
            Self::Orphaned => "orphaned — needs a human to re-place it",
            Self::MissingTarget => "target file is missing",
            Self::SymlinkEscape => "target resolves outside the work tree (symlink escape)",
        }
    }
}

/// A note deliberately left untouched, with why.
#[derive(Debug, Clone)]
pub struct ReanchorSkip {
    /// The note's target path.
    pub target: String,
    /// The skipped note's id.
    pub id: String,
    /// Why it was not re-anchored.
    pub reason: ReanchorSkipReason,
}

/// The outcome of a `reanchor` pass.
#[derive(Debug, Clone)]
pub struct ReanchorReport {
    /// Whether this was a preview (nothing written).
    pub dry_run: bool,
    /// Notes refreshed (or that would be).
    pub changed: Vec<ReanchorChange>,
    /// Notes left alone, with reasons.
    pub skipped: Vec<ReanchorSkip>,
    /// Notes already current (resolved to where they were captured).
    pub unchanged: usize,
}

/// Refreshes every note whose own region changed — it moved (resolves to a
/// different range) or its text was edited — so future queries start from the
/// current code. A note whose region is unchanged *and* still in place is left
/// untouched: rewriting it would refresh only ambient bundle state (the file's
/// line count, the HEAD commit, distant context) and rotate its
/// content-addressed id, churning the on-disk filename for no logical change
/// (the R2 regression this guards). An orphaned note is likewise left for a
/// human. With `dry_run`, computes the report but writes nothing.
///
/// # Errors
///
/// Returns [`Error::Invalid`]/[`Error::Io`] if the store or a target cannot
/// be read, or a refreshed note cannot be written.
pub fn reanchor(store: &Store, dry_run: bool) -> Result<ReanchorReport> {
    let workdir = store.workdir()?.to_path_buf();
    // Resolve once per invocation: every note's escape check compares against
    // the same canonical work tree, and `canonicalize` is a syscall worth
    // avoiding in the hot loop. `Ok(None)` if the work tree itself cannot be
    // canonicalised — fail-open then, leaving the guard to the per-note read.
    let canon_work = workdir.canonicalize().ok();
    let mut report = ReanchorReport {
        dry_run,
        changed: Vec::new(),
        skipped: Vec::new(),
        unchanged: 0,
    };

    // A malformed record cannot be parsed into a `Note`, so it never enters
    // this loop: it cannot be anchored against current code, and welding it
    // would be guesswork. It stays on disk, surfaced by `doctor`/`reindex`
    // rather than here — re-anchoring is not the place to report a corrupt
    // record. (`all_notes` skips it; it is not silently deleted.)
    for note in store.all_notes()?.notes {
        let abs = workdir.join(&note.target);
        // Mirror the save-time symlink guard: a target replaced by a symlink
        // after the note was saved (or a stored note whose target is itself
        // a symlink that was somehow committed past the save guard) must not
        // be read into a refreshed bundle, because doing so would pull the
        // symlink target's content — possibly from outside the work tree —
        // into a committed `.ynotes` record. Skipped, never refreshed.
        if let (Some(cw), Ok(canon_target)) = (canon_work.as_ref(), abs.canonicalize()) {
            if canon_target.strip_prefix(cw).is_err() {
                report.skipped.push(ReanchorSkip {
                    target: note.target.clone(),
                    id: note.id.clone(),
                    reason: ReanchorSkipReason::SymlinkEscape,
                });
                continue;
            }
        }
        let source = match SourceFile::read(&abs) {
            Ok(s) => s,
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                report.skipped.push(ReanchorSkip {
                    target: note.target.clone(),
                    id: note.id.clone(),
                    reason: ReanchorSkipReason::MissingTarget,
                });
                continue;
            }
            Err(e) => return Err(e),
        };

        let git = GitContext::discover(abs.parent().unwrap_or_else(|| std::path::Path::new(".")));
        let res = resolve(&note.bundle, &source, git.as_ref());

        let Some(to) = res.range else {
            // `range` is `None` exactly when the note orphaned: no content
            // rung located it, so re-anchoring would weld it to guesswork.
            report.skipped.push(ReanchorSkip {
                target: note.target.clone(),
                id: note.id.clone(),
                reason: ReanchorSkipReason::Orphaned,
            });
            continue;
        };

        let from = note.bundle.position.range;
        // Refresh only when the note's own region actually changed: it moved
        // (`to != from`) or its text was edited (the quote differs). Either is a
        // real change worth re-capturing every selector against. When the region
        // is unchanged *and* still in place, only ambient state could differ
        // (the file's line count, the HEAD commit, distant context); refreshing
        // that would rotate the content-addressed id and churn the on-disk
        // filename for no logical change — exactly the R2 case ("even when the
        // code is unchanged"). Leave it untouched, accepting that its git
        // baseline then ages (a mild trade: R1 still transports, from an older
        // commit).
        let region_unchanged = source.text(to).is_ok_and(|t| t == note.bundle.quote.exact);
        if to == from && region_unchanged {
            report.unchanged += 1;
            continue;
        }

        let fresh = SelectorBundle::capture_full(&source, to)?;
        let refreshed = note.clone().with_bundle(fresh)?;
        // Defensive backstop: a re-capture that somehow yields a bundle
        // identical to the stored one (hence the same id) must not be written —
        // `save` then `remove` on a single id would delete the record. Treat it
        // as unchanged. (With the guard above this is unreachable for a real
        // change, but it keeps the save/remove pair safe by construction.)
        if refreshed.id == note.id {
            report.unchanged += 1;
            continue;
        }
        if !dry_run {
            // Write the superseding note first, then retire the old one, so a
            // crash in between leaves both (index points at the new) rather
            // than losing the note.
            store.save(&refreshed)?;
            store.remove(&note)?;
        }
        report.changed.push(ReanchorChange {
            target: note.target.clone(),
            old_id: note.id.clone(),
            new_id: refreshed.id.clone(),
            from,
            to,
        });
    }

    Ok(report)
}
