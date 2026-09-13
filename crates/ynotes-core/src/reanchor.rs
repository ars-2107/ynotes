//! Persist resolved locations while preserving each note's review basis.
//!
//! Reads never write. This explicit pass replaces selectors only for locatable
//! regions and leaves orphans untouched to avoid losing their original anchors.

use std::path::Path;

use crate::anchor::{resolve, resolve_for_relocation};
use crate::error::{Error, Result};
use crate::git::{GitContext, RenameCache};
use crate::note::Note;
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
    /// Whether the note body still needs explicit review against current code.
    pub review_required: bool,
}

/// A note that was (or, in a dry run, would be) moved to a renamed file: its
/// target path changed because git detected the file was renamed, and a content
/// rung corroborated the region at the new path.
///
/// Distinct from [`ReanchorChange`] because a relocation moves the note between
/// files (its `target` changed), not just its position within one, the
/// `--json` agent contract surfaces it as its own `relocated[]` partition so a
/// consumer can branch on "the note moved files" separately.
#[derive(Debug, Clone)]
pub struct ReanchorRelocation {
    /// The target path the note was filed under before the rename.
    pub from_target: String,
    /// The target path it now lives at (the file's current name).
    pub to_target: String,
    /// Id before relocating.
    pub old_id: String,
    /// Id after (content-addressed over the new target and refreshed bundle).
    pub new_id: String,
    /// Where the region sat in the old file at save time.
    pub from: LineRange,
    /// Where it resolved to in the renamed file.
    pub to: LineRange,
    /// Whether the note body still needs explicit review against current code.
    pub review_required: bool,
}

/// Why a note was deliberately left untouched. A closed enum, deliberately,
/// the `--json` agent contract exposes this verbatim as `skipped[].reason_code`,
/// so consumers can branch on a stable code without parsing prose (invariant
/// #7). Adding a variant is a contract change; renaming one is a breaking
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReanchorSkipReason {
    /// No content rung located the region, re-anchoring would weld it to
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

    /// The free-form, user-facing explanation. Free to evolve for clarity,
    /// the stable code lives in [`Self::code`], so reworking the prose does
    /// not break the agent contract.
    #[must_use]
    pub fn human(self) -> &'static str {
        match self {
            Self::Orphaned => "orphaned, needs a human to re-place it",
            Self::MissingTarget => "target file is missing",
            Self::SymlinkEscape => "target resolves outside the work tree (symlink escape)",
        }
    }
}

/// A note whose selectors are already current but whose *body* has still not
/// been reviewed against the code they point at.
///
/// A note needing review appears in [`ReanchorChange`] or
/// [`ReanchorRelocation`] only on the pass that actually refreshes it. Every
/// later pass finds its region unchanged and in place, so without this list the
/// confirm remedy would disappear after one run while the note stayed `drifted`
/// indefinitely, the agent reruns `reanchor`, reads a clean report, and never
/// learns there is anything to confirm. Location and trust are tracked
/// separately (invariant #11), so the report has to say so separately too.
#[derive(Debug, Clone)]
pub struct ReanchorPending {
    /// The note's target path.
    pub target: String,
    /// The note's id, unchanged by this pass, so it is directly confirmable.
    pub id: String,
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
    /// Notes moved to a renamed file (or that would be), corroborated at the
    /// new path.
    pub relocated: Vec<ReanchorRelocation>,
    /// Notes left alone, with reasons.
    pub skipped: Vec<ReanchorSkip>,
    /// Notes whose selectors needed nothing but whose bodies are still
    /// unreviewed against the code they point at. Also counted in
    /// [`unchanged`](Self::unchanged): their location *is* current; only trust
    /// is outstanding.
    pub review_pending: Vec<ReanchorPending>,
    /// Notes already current (resolved to where they were captured).
    pub unchanged: usize,
}

/// Refreshes every note whose own region changed, it moved (resolves to a
/// different range) or its text was edited, so future queries start from the
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
    // canonicalised, fail-open then, leaving the guard to the per-note read.
    let canon_work = workdir.canonicalize().ok();
    let mut report = ReanchorReport {
        dry_run,
        changed: Vec::new(),
        relocated: Vec::new(),
        skipped: Vec::new(),
        review_pending: Vec::new(),
        unchanged: 0,
    };
    // Rename detection is a whole-tree diff per baseline commit; notes share
    // few commits, so memoise across the pass instead of paying it per note.
    let mut renames = RenameCache::default();

    // A malformed record cannot be parsed into a `Note`, so it never enters
    // this loop: it cannot be anchored against current code, and welding it
    // would be guesswork. It stays on disk, surfaced by `doctor`/`reindex`
    // rather than here, re-anchoring is not the place to report a corrupt
    // record. (`all_notes` skips it; it is not silently deleted.)
    for note in store.all_notes()?.notes {
        let abs = workdir.join(&note.target);
        // Mirror the save-time symlink guard: a target replaced by a symlink
        // after the note was saved (or a stored note whose target is itself
        // a symlink that was somehow committed past the save guard) must not
        // be read into a refreshed bundle, because doing so would pull the
        // symlink target's content, possibly from outside the work tree,
        // into a committed `.ynotes` record. Skipped, never refreshed.
        if let (Some(cw), Ok(canon_target)) = (canon_work.as_ref(), abs.canonicalize())
            && canon_target.strip_prefix(cw).is_err()
        {
            report.skipped.push(ReanchorSkip {
                target: note.target.clone(),
                id: note.id.clone(),
                reason: ReanchorSkipReason::SymlinkEscape,
            });
            continue;
        }
        let source = match SourceFile::read(&abs) {
            Ok(s) => s,
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                // The target file is gone. Before giving up, ask git whether it
                // was renamed and, if so, whether the region survived at the new
                // path, the sole way a note migrates across a refactor.
                match try_relocate(store, &note, &workdir, dry_run, &mut renames)? {
                    RelocateOutcome::Relocated(r) => report.relocated.push(r),
                    RelocateOutcome::Orphaned => report.skipped.push(ReanchorSkip {
                        target: note.target.clone(),
                        id: note.id.clone(),
                        reason: ReanchorSkipReason::Orphaned,
                    }),
                    RelocateOutcome::NotRenamed => report.skipped.push(ReanchorSkip {
                        target: note.target.clone(),
                        id: note.id.clone(),
                        reason: ReanchorSkipReason::MissingTarget,
                    }),
                }
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
        // filename for no logical change, exactly the R2 case ("even when the
        // code is unchanged"). Leave it untouched, accepting that its git
        // baseline then ages (a mild trade: R1 still transports, from an older
        // commit).
        let region_unchanged = source.text(to).is_ok_and(|t| t == note.bundle.quote.exact);
        if to == from && region_unchanged {
            report.unchanged += 1;
            // Selectors are current; the body may still not be. The pass that
            // refreshed this note reported `review_required` once and then
            // stopped seeing it, because from here on its region resolves
            // unchanged and in place. Report the outstanding review on every
            // later pass too, or rerunning `reanchor` looks clean while the
            // note stays `drifted` for good.
            if !source.text(to).is_ok_and(|text| note.review_matches(&text)) {
                report.review_pending.push(ReanchorPending {
                    target: note.target.clone(),
                    id: note.id.clone(),
                });
            }
            continue;
        }

        let fresh = SelectorBundle::capture_full(&source, to)?;
        let review_required = !source.text(to).is_ok_and(|text| note.review_matches(&text));
        let refreshed = note.clone().with_bundle(fresh)?;
        // Defensive backstop: a re-capture that somehow yields a bundle
        // identical to the stored one (hence the same id) must not be written,
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
            review_required,
        });
    }

    Ok(report)
}

/// The three outcomes of trying to follow a rename for a note whose target file
/// is missing on disk.
enum RelocateOutcome {
    /// git detected a rename and a content rung corroborated the region at the
    /// new path: the note was (or, in a dry run, would be) moved there.
    Relocated(ReanchorRelocation),
    /// git detected a rename but the region is gone at the new path (the file
    /// was renamed *and* the region deleted): declined, an honest orphan rather
    /// than a note welded to unrelated code (the wrong-file guard).
    Orphaned,
    /// No rename was detected, a genuine missing target.
    NotRenamed,
}

/// Follow a file rename for a note whose target is missing, corroborating the
/// region at the renamed path before moving anything.
///
/// The rename signal alone never moves a note: git proposes the destination,
/// and only a content rung locating the region there (`resolve` returning a
/// range) confirms the move. A destination where the region is gone yields
/// [`RelocateOutcome::Orphaned`], never a relocation, the guard that keeps a
/// rename from welding a note onto the wrong lines of the renamed file.
///
/// Detection needs the note's git selector (its baseline commit and old path);
/// a note untracked at save time carries none, so it cannot be followed and is
/// reported as a genuine missing target.
///
/// # Errors
///
/// [`Error::Invalid`]/[`Error::Io`] if the renamed file cannot be captured into
/// a fresh bundle or the relocated note cannot be written.
fn try_relocate(
    store: &Store,
    note: &Note,
    workdir: &Path,
    dry_run: bool,
    renames: &mut RenameCache,
) -> Result<RelocateOutcome> {
    // No git selector ⇒ untracked at save ⇒ no rename signal to follow.
    let Some(gitsel) = note.bundle.git.as_ref() else {
        return Ok(RelocateOutcome::NotRenamed);
    };
    let Some(ctx) = GitContext::discover(workdir) else {
        return Ok(RelocateOutcome::NotRenamed);
    };
    // git reports the destination relative to the *git* root, which is not
    // necessarily the store root, so rebuild the absolute path from the git
    // root before translating it back to a store target.
    let Some(new_git_rel) = renames.lookup(&ctx, &gitsel.commit, &gitsel.path) else {
        return Ok(RelocateOutcome::NotRenamed);
    };
    let new_abs = ctx.root().join(&new_git_rel);

    // `relativize` resolves symlinks and refuses a path outside the work tree,
    // so it doubles as the store-target translation *and* the symlink-escape
    // guard: a rename that lands outside the store (or via an escaping symlink)
    // returns `OutsideStore`, and we decline rather than read foreign content
    // into a committed bundle.
    let Ok(new_target) = store.relativize(&new_abs) else {
        return Ok(RelocateOutcome::NotRenamed);
    };
    // git named a destination but it is not a readable text file here, do not
    // guess; leave the note as a missing target.
    let Ok(new_source) = SourceFile::read(&new_abs) else {
        return Ok(RelocateOutcome::NotRenamed);
    };

    // Corroborate in relocation mode (`resolve_for_relocation`): content rungs
    // only, and no "did not move" shortcut. Two reasons the ordinary resolve is
    // wrong here. First, the note's git selector still names the renamed-away
    // *old* path, so the git rung would transport onto the deletion point and
    // could falsely witness a moved fuzzy match near the top of the new file,
    // welding the note to unrelated code (invariant #10), hence no git context.
    // Second, a fuzzy hit at the note's *saved* line numbers is not evidence in a
    // *different* file: a rename that replaced the region in place would otherwise
    // be accepted, so the saved-range exemption is dropped and such a hit needs an
    // independent witness or orphans. Only rungs that search the new file's actual
    // text may confirm the region is really here.
    let Some(to) = resolve_for_relocation(&note.bundle, &new_source).range else {
        // Renamed, but the region is gone at the new path: honest orphan.
        return Ok(RelocateOutcome::Orphaned);
    };

    let fresh = SelectorBundle::capture_full(&new_source, to)?;
    let review_required = !new_source
        .text(to)
        .is_ok_and(|text| note.review_matches(&text));
    // Migrate only to a *committed* destination. `capture_full` attaches a git
    // selector exactly when the new path is tracked at `HEAD`; its absence means
    // the rename is only staged, not committed. Relocating there would produce a
    // note with no git baseline, unable to follow a later rename and without the
    // R1 transport rung, and reanchor's unchanged-region skip would never heal
    // it after the commit. Defer instead: leave the note put (the `query`
    // self-heal already serves reads at the new path) until the rename is
    // committed, when this pass migrates it with a baseline intact.
    if fresh.git.is_none() {
        return Ok(RelocateOutcome::NotRenamed);
    }
    let relocated = note.clone().relocated(new_target.clone(), fresh)?;
    if !dry_run {
        // Write the relocated note first, then retire the old one, so a crash
        // in between leaves both (indexed at the new path) rather than losing
        // the note, the same ordering the in-place refresh path uses.
        store.save(&relocated)?;
        store.remove(note)?;
    }
    Ok(RelocateOutcome::Relocated(ReanchorRelocation {
        from_target: note.target.clone(),
        to_target: new_target,
        old_id: note.id.clone(),
        new_id: relocated.id.clone(),
        from: note.bundle.position.range,
        to,
        review_required,
    }))
}
