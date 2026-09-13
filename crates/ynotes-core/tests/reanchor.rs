//! Reanchor safety, idempotence, review preservation, and dirty-tree transport.
//! Orphans retain their records and selectors.

use std::path::Path;
use std::process::Command;

use ynotes::{
    AnchorStatus, GitContext, LineRange, LineSpec, Note, Rung, RungResult, Scope, SelectorBundle,
    SourceFile, Store, confirm, query, reanchor, resolve,
};
// Only the unix-gated read-only-store test matches on skip reasons; importing
// this unconditionally is an unused-import error on Windows under -D warnings.
#[cfg(unix)]
use ynotes::ReanchorSkipReason;

/// Saves a Range note for `range` of the file at `abs`.
fn save_note(store: &Store, abs: &Path, range: LineRange, body: &str) -> Note {
    let src = SourceFile::read(abs).unwrap();
    let bundle = SelectorBundle::capture_full(&src, range).unwrap();
    let note = Note::new(
        store.relativize(abs).unwrap(),
        Scope::Range,
        bundle,
        body.to_owned(),
    )
    .unwrap();
    store.save(&note).unwrap();
    note
}

/// A record path that enumerates but is gone by the time it is read is an
/// absence, not a malformed record. `reanchor` supersedes by saving then
/// removing, and any `delete` can land between a reader's enumeration and its
/// read, so this window is expected rather than exceptional.
///
/// A dangling link reproduces the state deterministically. A thread race does
/// not: the window is narrow enough that a spawned deleter reliably loses,
/// which would make the test pass whether the classification is right or not.
#[cfg(unix)]
#[test]
fn a_record_that_vanishes_before_it_is_read_is_not_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("lib.rs");
    std::fs::write(&file, "fn one() {}\nfn two() {}\nfn three() {}\n").unwrap();
    save_note(&store, &file, LineRange::new(1, 2).unwrap(), "why");

    // A second record path pointing at a file that does not exist: reading it
    // fails with `NotFound`, exactly as a concurrently deleted record does.
    let fanout = store.root().join("notes").join("aa");
    std::fs::create_dir_all(&fanout).unwrap();
    std::os::unix::fs::symlink(
        dir.path().join("gone.json"),
        fanout.join(format!("{}.json", "b".repeat(62))),
    )
    .unwrap();

    let scan = store.notes_for("lib.rs").unwrap();
    assert!(
        scan.malformed.is_empty(),
        "a record that is simply gone is an absence, not corruption: {:?}",
        scan.malformed
    );
    assert_eq!(
        scan.notes.len(),
        1,
        "the healthy note must still come back alongside it"
    );
}

/// `confirm` advances trust irreversibly, so a region the ladder only
/// *resembles* must never become the new review basis. Without a git baseline
/// (no repository here) or a parseable syntax tree (`.txt`), an edited region
/// resolves through the fuzzy rung alone, the guess that would otherwise
/// record a body as reviewed against code nobody read.
#[test]
fn confirm_refuses_a_region_located_only_by_similarity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "alpha one\nbravo two\ncharlie three\ndelta four\n").unwrap();
    let note = save_note(
        &store,
        &file,
        LineRange::new(1, 3).unwrap(),
        "why this block",
    );

    // Similar enough for fuzzy to land on it, different enough that no exact
    // quote survives.
    std::fs::write(&file, "alpha ONE\nbravo two\ncharlie THREE\ndelta four\n").unwrap();

    let err = confirm(&store, note).unwrap_err();
    assert!(
        matches!(err, ynotes::Error::Unidentified { .. }),
        "similarity must not advance trust: {err}"
    );
    assert!(
        err.to_string().contains("ynotes reanchor"),
        "the refusal must name its remedy: {err}"
    );
}

#[test]
fn reanchor_refreshes_a_drifted_note_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("lib.rs");

    std::fs::write(
        &file,
        "fn one() {}\nfn target() {\n    work();\n}\nfn z() {}\n",
    )
    .unwrap();
    let original = save_note(&store, &file, LineRange::new(2, 4).unwrap(), "hot path");

    // Shift the region down by two lines.
    std::fs::write(
        &file,
        "// a\n// b\nfn one() {}\nfn target() {\n    work();\n}\nfn z() {}\n",
    )
    .unwrap();

    // Dry run: reports the change, writes nothing.
    let preview = reanchor(&store, true).unwrap();
    assert_eq!(preview.changed.len(), 1);
    assert!(
        store
            .notes_for("lib.rs")
            .unwrap()
            .notes
            .iter()
            .any(|n| n.id == original.id),
        "dry run must not modify the store"
    );

    // Apply: the old note is superseded by a new content-addressed id.
    let applied = reanchor(&store, false).unwrap();
    assert_eq!(applied.changed.len(), 1);
    assert!(
        !applied.changed[0].review_required,
        "moving unchanged code does not require content review"
    );
    let live = store.notes_for("lib.rs").unwrap().notes;
    assert_eq!(live.len(), 1, "superseded, not duplicated");
    assert_ne!(live[0].id, original.id, "id reflects the refreshed bundle");
    assert_eq!(live[0].body, "hot path");
    assert_eq!(live[0].created_at, original.created_at, "provenance kept");

    // Idempotent: nothing left to do.
    let again = reanchor(&store, false).unwrap();
    assert_eq!(again.changed.len(), 0);
    assert_eq!(again.unchanged, 1);
}

/// R2 regression: `reanchor` must not rewrite a note that did not move. The id
/// is content-addressed over the whole selector bundle, so volatile bundle
/// state, here the file's line count, after appending a line far below the
/// note, used to rotate the id and churn the on-disk filename even though the
/// region never moved. `reanchor` now refreshes only a note that actually
/// relocated (resolved range != stored range), so an unmoved note keeps its id
/// and its file.
#[test]
fn reanchor_leaves_an_unmoved_note_untouched_when_only_the_file_grew() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("lib.rs");

    // A distinctive region at the top, with enough lines below it that
    // appending at the end touches neither the region nor its 3-line suffix.
    std::fs::write(
        &file,
        "fn target_one() {}\nfn target_two() {}\nfn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\n",
    )
    .unwrap();
    let original = save_note(&store, &file, LineRange::new(1, 2).unwrap(), "top region");

    // Append a line at the very end: the note's region (1:2) does not move, but
    // the file's line count changes, the volatile bundle state that used to
    // rotate the id.
    std::fs::write(
        &file,
        "fn target_one() {}\nfn target_two() {}\nfn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn e() {}\nfn appended() {}\n",
    )
    .unwrap();

    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.changed.len(), 0, "an unmoved note is not rewritten");
    assert_eq!(report.unchanged, 1, "it is reported as already current");

    let live = store.notes_for("lib.rs").unwrap().notes;
    assert_eq!(
        live.len(),
        1,
        "no churn: one note, not a new id beside the old"
    );
    assert_eq!(live[0].id, original.id, "the unmoved note keeps its id");
}

/// Re-anchoring may refresh location selectors after an in-place edit, but it
/// must not silently claim the note body was reviewed against that edit. The
/// note therefore remains drifted until an explicit update/confirmation moves
/// its review basis to the current code.
#[test]
fn reanchor_preserves_stale_context_after_an_in_place_edit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("lib.rs");

    std::fs::write(
        &file,
        "fn one() {}\nfn target() {\n    work();\n}\nfn z() {}\n",
    )
    .unwrap();
    let original = save_note(&store, &file, LineRange::new(2, 4).unwrap(), "hot path");

    // Edit the body of the noted region without changing its line count: the
    // note still resolves to 2:4 (structural/position), but its text changed.
    std::fs::write(
        &file,
        "fn one() {}\nfn target() {\n    work_v2();\n}\nfn z() {}\n",
    )
    .unwrap();

    let report = reanchor(&store, false).unwrap();
    assert_eq!(
        report.changed.len(),
        1,
        "an in-place content edit is refreshed"
    );
    assert!(
        report.changed[0].review_required,
        "the maintenance report must directly flag stale prose"
    );
    assert_eq!(report.unchanged, 0);

    let live = store.notes_for("lib.rs").unwrap().notes;
    assert_eq!(live.len(), 1, "superseded, not duplicated");
    assert_ne!(
        live[0].id, original.id,
        "the refreshed bundle yields a new id"
    );
    assert!(
        live[0].bundle.quote.exact.contains("work_v2"),
        "location selectors refresh against the current code: {}",
        live[0].bundle.quote.exact
    );
    assert_eq!(live[0].created_at, original.created_at, "provenance kept");

    let result = query(&store, &file, LineSpec::Whole).unwrap();
    assert_eq!(result.matched.len(), 1);
    assert!(
        matches!(
            result.matched[0].resolution.status,
            AnchorStatus::Drifted { .. }
        ),
        "reanchor must not clear staleness without reviewing the note body"
    );

    let stale_id = result.matched[0].note.id.clone();
    let confirmed = confirm(&store, result.matched[0].note.clone()).unwrap();
    assert!(
        confirmed.changed,
        "advancing the review basis rotates the id"
    );
    assert_eq!(confirmed.previous_id, stale_id);

    let reviewed = query(&store, &file, LineSpec::Whole).unwrap();
    assert_eq!(reviewed.matched.len(), 1);
    assert!(
        matches!(
            reviewed.matched[0].resolution.status,
            AnchorStatus::Anchored
        ),
        "explicit confirmation is what clears staleness"
    );
}

#[test]
fn confirm_refuses_an_orphan_instead_of_trusting_its_saved_position() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("lib.rs");
    std::fs::write(&file, "fn target() { unique_work(); }\n").unwrap();
    let note = save_note(&store, &file, LineRange::new(1, 1).unwrap(), "why");
    std::fs::write(&file, "fn unrelated() {}\n").unwrap();

    let err = confirm(&store, note).expect_err("an orphan cannot be confirmed");
    assert!(matches!(err, ynotes::Error::Unconfirmable { .. }));
}

#[test]
fn reanchor_never_touches_an_orphaned_note() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("a.rs");

    std::fs::write(&file, "fn keep() {}\nfn doomed() {\n    gone();\n}\n").unwrap();
    let note = save_note(&store, &file, LineRange::new(2, 4).unwrap(), "context");

    // Replace the region with entirely unrelated content ⇒ orphaned.
    std::fs::write(
        &file,
        "const A: u8 = 1;\ntype B = u16;\nstatic C: u8 = 2;\n",
    )
    .unwrap();

    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.changed.len(), 0);
    assert_eq!(report.skipped.len(), 1, "orphaned ⇒ left for a human");
    let live = store.notes_for("a.rs").unwrap().notes;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, note.id, "untouched: same id, same anchor");
}

#[test]
fn deleted_duplicate_stays_orphaned_through_maintenance_and_git_commits() {
    for legacy in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(git(root, &["init", "-q"]));
        assert!(git(root, &["config", "user.email", "test@example.invalid"]));
        assert!(git(root, &["config", "user.name", "Test"]));
        let file = root.join("routes.js");
        std::fs::write(&file, "export function legacyRoute() {\n  return 42;\n}\n\nexport function newRoute() {\n  return 42;\n}\n").unwrap();
        assert!(git(root, &["add", "routes.js"]));
        assert!(git(root, &["commit", "-qm", "two routes"]));
        let store = Store::init(root).unwrap();
        let source = SourceFile::read(&file).unwrap();
        let mut bundle =
            SelectorBundle::capture_full(&source, LineRange::new(2, 2).unwrap()).unwrap();
        if legacy {
            bundle.quote.occurrences_at_capture = None;
        }
        let note = Note::new(
            "routes.js".into(),
            Scope::Range,
            bundle,
            "Only legacyRoute must preserve 42 for old clients.".into(),
        )
        .unwrap();
        store.save(&note).unwrap();
        let before = query(&store, &file, LineSpec::Whole).unwrap();
        assert!(matches!(
            before.matched[0].resolution.status,
            AnchorStatus::Anchored
        ));

        std::fs::write(&file, "export function newRoute() {\n  return 42;\n}\n").unwrap();
        for committed in [false, true] {
            if committed {
                assert!(git(root, &["add", "routes.js"]));
                assert!(git(root, &["commit", "-qm", "delete legacy route"]));
            }
            for _ in 0..2 {
                let result = query(&store, &file, LineSpec::Whole).unwrap();
                assert!(result.matched.is_empty());
                assert_eq!(
                    result.orphaned.len(),
                    1,
                    "the historical note remains visible"
                );
                assert!(matches!(
                    result.orphaned[0].resolution.status,
                    AnchorStatus::Orphaned { .. }
                ));
                assert_eq!(result.orphaned[0].resolution.range, None);
                let report = reanchor(&store, false).unwrap();
                assert!(report.changed.is_empty());
                assert_eq!(report.skipped.len(), 1);
                assert!(matches!(
                    confirm(&store, note.clone()),
                    Err(ynotes::Error::Unconfirmable { .. })
                ));
                let live = store.notes_for("routes.js").unwrap().notes;
                assert_eq!(live.len(), 1);
                assert_eq!(live[0].id, note.id);
            }
        }
    }
}

/// Runs a git command in `dir`, reporting whether it succeeded.
fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The range R1 (the git rung) resolved `bundle` to in `source`, or `None`
/// if the git rung did not hit.
fn git_hit(bundle: &SelectorBundle, source: &SourceFile, ctx: &GitContext) -> Option<LineRange> {
    resolve(bundle, source, Some(ctx))
        .rungs
        .into_iter()
        .find(|o| o.rung == Rung::Git)
        .and_then(|o| match o.result {
            RungResult::Hit { range, .. } => Some(range),
            _ => None,
        })
}

/// The repo's current HEAD commit id (full SHA), trimmed.
fn git_head(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// `reanchor` against a *dirty* working tree must not desync the git rung.
/// The git selector's baseline is captured in its commit's own coordinates,
/// so after a dirty reanchor the git rung still agrees with the resolved
/// range, and keeps agreeing once the pending edit is committed.
#[test]
fn reanchor_on_a_dirty_tree_keeps_the_git_rung_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("s.rs");
    std::fs::write(
        &file,
        "fn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n    let q = y;\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let store = Store::init(root).unwrap();
    // A note on the whole of `beta()` (lines 5:8), saved against a clean tree.
    save_note(&store, &file, LineRange::new(5, 8).unwrap(), "beta region");

    // Insert three lines at the top and do NOT commit: this is the documented
    // `reanchor` workflow, you run it *after* editing, tree still dirty.
    std::fs::write(
        &file,
        "// h1\n// h2\n// h3\nfn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n    let q = y;\n}\n",
    )
    .unwrap();

    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.changed.len(), 1, "the drifted note is refreshed");

    let ctx = GitContext::discover(root).expect("repo discovered");
    let refreshed = store
        .notes_for("s.rs")
        .unwrap()
        .notes
        .pop()
        .expect("one note");

    // `beta()` is now at 8:11. The git rung must agree, not report 11:14,
    // which is 8:11 plus the three-line skew uncommitted at reanchor time.
    let dirty = SourceFile::read(&file).unwrap();
    let res = resolve(&refreshed.bundle, &dirty, Some(&ctx));
    assert_eq!(
        res.range,
        Some(LineRange::new(8, 11).unwrap()),
        "the region resolves to its true location"
    );
    assert_eq!(
        git_hit(&refreshed.bundle, &dirty, &ctx),
        res.range,
        "the git rung agrees with the resolved range, no dirty-diff skew"
    );

    // Committing the pending edit must not reintroduce the skew either.
    git(root, &["add", "s.rs"]);
    git(root, &["commit", "-q", "-m", "add header"]);
    let committed = SourceFile::read(&file).unwrap();
    assert_eq!(
        git_hit(&refreshed.bundle, &committed, &ctx),
        Some(LineRange::new(8, 11).unwrap()),
        "the git rung stays correct after the edit is committed"
    );
}

/// R2 trade, verified: when `reanchor` leaves an unmoved, unedited note alone,
/// its git baseline *ages*, it keeps pointing at the commit it was captured
/// against even as HEAD advances. This proves the ageing is safe: the note's id
/// does not churn, and it still resolves correctly, with the git rung still
/// transporting it across the now-older baseline.
#[test]
fn an_unmoved_notes_git_baseline_ages_but_it_still_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("s.rs");
    std::fs::write(
        &file,
        "fn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n    let q = y;\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }
    let baseline = git_head(root);

    let store = Store::init(root).unwrap();
    // A note on the whole of `beta()` (lines 5:8), anchored against `baseline`.
    let saved = save_note(&store, &file, LineRange::new(5, 8).unwrap(), "beta region");
    assert_eq!(
        saved.bundle.git.as_ref().map(|g| g.commit.as_str()),
        Some(baseline.as_str()),
        "the note is anchored to the commit it was saved against"
    );

    // Advance HEAD with an unrelated commit: append `gamma()` far below
    // `beta()`, so `beta()` does not move.
    std::fs::write(
        &file,
        "fn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n    let q = y;\n}\n\nfn gamma() {\n    let z = 3;\n}\n",
    )
    .unwrap();
    git(root, &["add", "s.rs"]);
    git(root, &["commit", "-q", "-m", "add gamma"]);
    assert_ne!(baseline, git_head(root), "HEAD advanced");

    // `beta()` is unmoved and unedited, so `reanchor` leaves the note alone,
    // its baseline is NOT refreshed to the new HEAD.
    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.changed.len(), 0, "the unmoved note is not rewritten");
    assert_eq!(report.unchanged, 1);

    let live = store
        .notes_for("s.rs")
        .unwrap()
        .notes
        .pop()
        .expect("one note");
    assert_eq!(live.id, saved.id, "id unchanged, no churn");
    assert_eq!(
        live.bundle.git.as_ref().map(|g| g.commit.as_str()),
        Some(baseline.as_str()),
        "baseline aged: still the original commit, not the new HEAD"
    );

    // Despite the aged baseline, the note still resolves to `beta()`'s true
    // location. The git rung still transports from the older baseline (it
    // hits), but ageing costs it some precision, appending `gamma()` right
    // after the region makes R1 over-extend to 5:12; the ladder reconciles that
    // against the other rungs back to the correct 5:8. That the combined verdict
    // is right *because* the rungs disagree-and-vote is exactly why the ageing
    // trade is safe.
    let ctx = GitContext::discover(root).expect("repo discovered");
    let src = SourceFile::read(&file).unwrap();
    let res = resolve(&live.bundle, &src, Some(&ctx));
    assert_eq!(
        res.range,
        Some(LineRange::new(5, 8).unwrap()),
        "the note still resolves to its region after the baseline aged"
    );
    assert!(
        git_hit(&live.bundle, &src, &ctx).is_some(),
        "R1 still transports (hits) from the older baseline commit"
    );
}

/// A note's target replaced by a symlink to an outside file must not be
/// refreshed by `reanchor`. The save-time guard catches the obvious case
/// (saving directly against a symlink); this catches the post-save attack,
/// a note saved against a real file whose path is later swapped for a
/// symlink. Without this, `reanchor` would read the symlink target's
/// content into a fresh selector bundle and commit it into `.ynotes`,
/// leaking whatever lay behind the link.
///
/// Unix-only: meaningful only where symlinks behave this way.
#[cfg(unix)]
#[test]
fn reanchor_refuses_a_target_that_was_replaced_by_an_escaping_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = Store::init(root).unwrap();

    // Save a perfectly ordinary note against a real file.
    let file = root.join("real.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
    save_note(
        &store,
        &file,
        LineRange::new(1, 3).unwrap(),
        "ordinary note",
    );

    // Then replace that file with a symlink whose target escapes the work
    // tree. The note is unchanged on disk; `reanchor` is the operation that
    // must notice and refuse.
    std::fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", &file).expect("symlink");

    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.changed.len(), 0, "no notes refreshed");
    assert_eq!(report.skipped.len(), 1, "the symlinked note is skipped");
    assert_eq!(
        report.skipped[0].reason,
        ReanchorSkipReason::SymlinkEscape,
        "skip reason classifies as the symlink escape",
    );
}

/// The companion case: a target swapped for a symlink whose canonical path
/// stays *inside* the work tree must still be refreshed by `reanchor`. The
/// save-time guard already exercises both branches; without this test, a
/// regression that over-applied the reanchor guard (refusing all symlinks)
/// would go unnoticed. Vendored dirs and monorepo workspace links rely on
/// this branch keeping working.
#[cfg(unix)]
#[test]
fn reanchor_allows_a_target_replaced_by_an_intra_repo_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let store = Store::init(root).unwrap();

    // Save against a real file whose content will *also* live at a second
    // path inside the work tree; the symlink will point to that second path.
    let original = root.join("page.md");
    std::fs::write(&original, "# section\nalpha\nbeta\ngamma\n").unwrap();
    save_note(
        &store,
        &original,
        LineRange::new(1, 4).unwrap(),
        "page note",
    );

    // Replace `page.md` with a symlink to `alias.md`, whose contents the
    // resolver will read instead. Both paths are inside the work tree, so the
    // guard must allow it; the bundle's selectors still match (identical
    // text), so `reanchor` reports "already current".
    let alias = root.join("alias.md");
    std::fs::write(&alias, "# section\nalpha\nbeta\ngamma\n").unwrap();
    std::fs::remove_file(&original).unwrap();
    std::os::unix::fs::symlink(&alias, &original).expect("intra-repo symlink");

    let report = reanchor(&store, false).unwrap();
    assert_eq!(
        report.skipped.len(),
        0,
        "intra-repo symlink must not trigger the escape guard"
    );
    // Identical content through the symlink ⇒ identical bundle ⇒ identical
    // id ⇒ the `unchanged` branch specifically. Pin it down; a `changed`
    // hit here would be a regression (a stable-content reanchor producing
    // a fresh id) masquerading as success.
    assert_eq!(report.unchanged, 1, "stable-content note must be unchanged");
    assert!(
        report.changed.is_empty(),
        "no refresh should be produced for identical content: got {:?}",
        report.changed
    );
}

/// The documented remedy for a drifted note is `reanchor`, then `confirm`. An
/// agent that reruns `reanchor` first, after another agent already ran it, or
/// simply because rerunning maintenance is cheap, used to get a spotlessly
/// clean report: the note's region now resolves unchanged and in place, so it
/// fell into the `unchanged` tally and no longer appeared anywhere naming an
/// id. The note stayed `drifted` forever with nothing pointing at it.
#[test]
fn a_second_reanchor_pass_still_names_the_note_awaiting_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let file = dir.path().join("lib.rs");

    std::fs::write(
        &file,
        "fn one() {}\nfn target() {\n    work();\n}\nfn z() {}\n",
    )
    .unwrap();
    save_note(&store, &file, LineRange::new(2, 4).unwrap(), "hot path");

    std::fs::write(
        &file,
        "fn one() {}\nfn target() {\n    work_v2();\n}\nfn z() {}\n",
    )
    .unwrap();

    let first = reanchor(&store, false).unwrap();
    assert!(
        first.changed[0].review_required,
        "the refreshing pass flags the outstanding review"
    );
    assert!(
        first.review_pending.is_empty(),
        "it is already named in changed[]; do not report it twice"
    );

    let second = reanchor(&store, false).unwrap();
    assert!(
        second.changed.is_empty() && second.relocated.is_empty(),
        "selectors are current now, so nothing is rewritten"
    );
    assert_eq!(
        second.review_pending.len(),
        1,
        "the outstanding review must survive every later pass"
    );
    assert_eq!(
        second.unchanged, 1,
        "its location is current; its body is not"
    );

    // The id is live, so it goes straight to `confirm` with no lookup step.
    let pending = &second.review_pending[0];
    assert_eq!(pending.target, "lib.rs");
    let note = match store.match_id_prefix(&pending.id).unwrap() {
        ynotes::IdMatch::One(note) => note,
        other => panic!("review_pending must carry a live id, got {other:?}"),
    };
    confirm(&store, note).expect("the reported id confirms directly");

    let third = reanchor(&store, false).unwrap();
    assert!(
        third.review_pending.is_empty(),
        "a confirmed note stops being reported"
    );
}
