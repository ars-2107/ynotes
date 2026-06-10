//! R1 end-to-end: a note committed at one revision still resolves after the
//! working tree is edited, because git transports the range through the diff
//! — and a region git can only transport onto a deletion point still orphans
//! honestly rather than anchoring there.
//!
//! Skips itself cleanly if no `git` binary is present, so the suite stays
//! green on a machine without git while still covering R1 where it matters.

use std::path::Path;
use std::process::Command;

use ynotes::{
    AnchorStatus, GitContext, GitSelector, LineRange, SelectorBundle, SourceFile, resolve,
};

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn git_rung_transports_a_range_across_a_working_tree_edit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("lib.rs");
    std::fs::write(
        &file,
        "fn one() {}\nfn target() {\n    body();\n}\nfn last() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let ctx = GitContext::discover(root).expect("repo discovered");
    let commit = ctx.head_commit().expect("HEAD commit");
    let rel = ctx.relativize(&file).expect("relative path");

    // Note targets `fn target` at lines 2:4 of the committed file.
    let committed = SourceFile::read(&file).unwrap();
    let bundle = SelectorBundle::capture(&committed, LineRange::new(2, 4).unwrap())
        .unwrap()
        .with_git(GitSelector {
            commit,
            path: rel,
            range: Some(LineRange::new(2, 4).unwrap()),
        });

    // Insert three lines above the region in the working tree (uncommitted).
    std::fs::write(
        &file,
        "// a\n// b\n// c\nfn one() {}\nfn target() {\n    body();\n}\nfn last() {}\n",
    )
    .unwrap();
    let edited = SourceFile::read(&file).unwrap();

    let res = resolve(&bundle, &edited, Some(&ctx));

    // The region moved from 2:4 to 5:7; R1 must carry it there.
    assert_eq!(res.range, Some(LineRange::new(5, 7).unwrap()));
}

/// A region whose enclosing construct is deleted resolves `orphaned`, not
/// `drifted` — even though git happily transports the now-deleted range onto
/// the deletion point. A git (or position) hit is not, on its own, evidence
/// the region still exists; only the content rungs are. Regression test for
/// the deleted-region mis-anchor.
#[test]
fn a_deleted_region_orphans_even_when_git_collapses_it() {
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
        "fn keep() {\n    1;\n}\n\nfn doomed() {\n    let secret = 99;\n    compute(secret);\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let ctx = GitContext::discover(root).expect("repo discovered");
    // The note covers the whole of `doomed()` — lines 5:8 of the commit.
    let committed = SourceFile::read(&file).unwrap();
    let bundle = SelectorBundle::capture_full(&committed, LineRange::new(5, 8).unwrap()).unwrap();
    assert!(bundle.git.is_some(), "a tracked file gets a git selector");

    // Delete `doomed()` entirely and commit, so git has a real deletion hunk.
    std::fs::write(&file, "fn keep() {\n    1;\n}\n").unwrap();
    git(root, &["add", "s.rs"]);
    git(root, &["commit", "-q", "-m", "delete doomed"]);

    let after = SourceFile::read(&file).unwrap();
    let res = resolve(&bundle, &after, Some(&ctx));

    assert!(
        matches!(res.status, AnchorStatus::Orphaned { .. }),
        "a deleted region must orphan, not drift: {:?}",
        res.status
    );
    assert_eq!(res.range, None, "an orphan has no resolved range");
}

/// An untracked file — one never `git add`-ed, absent from HEAD — gets no git
/// selector. Its git baseline is empty (`git diff HEAD -- <path>` shows
/// nothing), so R1 would report a spurious result; `capture_full` must instead
/// leave `git: None` and let the content rungs carry resolution.
#[test]
fn an_untracked_file_gets_no_git_selector() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    // A committed file exists, so HEAD is non-empty and a `GitContext` is
    // discoverable — but `u.rs` itself is never `git add`-ed.
    let tracked = root.join("tracked.rs");
    std::fs::write(&tracked, "fn anchor() {}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let untracked = root.join("u.rs");
    std::fs::write(&untracked, "fn one() {}\nfn target() {\n    body();\n}\n").unwrap();

    let file = SourceFile::read(&untracked).unwrap();
    let bundle = SelectorBundle::capture_full(&file, LineRange::new(2, 4).unwrap()).unwrap();

    assert!(
        bundle.git.is_none(),
        "an untracked file must not get a git selector: {:?}",
        bundle.git
    );
}

/// The complement of the above: a committed (tracked) file still gets a git
/// selector from `capture_full`, so the guard does not over-fire.
#[test]
fn a_tracked_file_still_gets_a_git_selector() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("t.rs");
    std::fs::write(&file, "fn one() {}\nfn target() {\n    body();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let committed = SourceFile::read(&file).unwrap();
    let bundle = SelectorBundle::capture_full(&committed, LineRange::new(2, 4).unwrap()).unwrap();

    assert!(
        bundle.git.is_some(),
        "a tracked file must still get a git selector",
    );
}

/// A region whose contents were edited in place AND whose position shifted
/// (a comment line inserted above) must resolve `drifted`, not `orphaned`.
/// The resolver's fuzzy-only check demands an *independent* witness when
/// fuzzy moved off the saved range; in a real repo, the git transport rung
/// provides exactly that — it carries the original range through the diff to
/// the new location, overlapping fuzzy's hit. Without this corroboration the
/// resolver would (correctly) orphan a fuzzy-only-and-moved hit; with it the
/// note drifts honestly. Regression test for both branches of that rule.
#[test]
fn fuzzy_drifts_when_git_corroborates_a_moved_in_place_edit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("svc.rs");
    std::fs::write(
        &file,
        "fn handler(req: Request) -> Response {\n\
         \x20   let user = authenticate(req);\n\
         \x20   let data = load(user.id);\n\
         \x20   render(data)\n\
         }\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let ctx = GitContext::discover(root).expect("repo discovered");
    let committed = SourceFile::read(&file).unwrap();
    // Note covers the whole function at 1:5.
    let bundle = SelectorBundle::capture_full(&committed, LineRange::new(1, 5).unwrap()).unwrap();
    assert!(bundle.git.is_some(), "tracked file gets a git selector");

    // Edit the body of one line (`load` → `load_cached`) and prepend a
    // comment, all in the working tree. Quote misses (text changed). Fuzzy
    // hits at 2:6 — but that is *moved* from the saved 1:5, so the rule
    // demands an independent witness. Git transport supplies it.
    std::fs::write(
        &file,
        "// new comment\n\
         fn handler(req: Request) -> Response {\n\
         \x20   let user = authenticate(req);\n\
         \x20   let data = load_cached(user.id, ttl);\n\
         \x20   render(data)\n\
         }\n",
    )
    .unwrap();
    let edited = SourceFile::read(&file).unwrap();

    let res = resolve(&bundle, &edited, Some(&ctx));

    assert!(
        matches!(res.status, AnchorStatus::Drifted { .. }),
        "git corroborates fuzzy's moved hit ⇒ drifted, not orphaned: {:?}",
        res.status
    );
    let r = res.range.expect("region relocated, not orphaned");
    assert!(
        r.start() <= 2 && r.end() >= 6,
        "expected to cover the moved+edited block, got {}:{}",
        r.start(),
        r.end()
    );
}

/// The pure fuzzy-only-and-moved case: an unparsed file (so the structural
/// rung is `Skipped` and cannot vouch), no git context, an edit that shifts
/// content by one line and changes it in place. Fuzzy hits at the new
/// location; nothing else does. Under the tightened rule that must orphan —
/// otherwise an arbitrary fuzzy guess could become a confident `drifted`
/// resolution that `reanchor` then welds in place.
///
/// In a real repo this same edit drifts (git transports the range, see
/// [`fuzzy_drifts_when_git_corroborates_a_moved_in_place_edit`] above); in a
/// language tree-sitter parses, structural likely corroborates too. The
/// no-git + unparsed combination is rare in production but it is the exact
/// branch of the rule that needed an explicit regression test.
#[test]
fn fuzzy_orphans_a_moved_in_place_edit_without_any_corroboration() {
    let dir = tempfile::tempdir().unwrap();
    // Note: no `git init`. `.txt` so tree-sitter does not parse it ⇒ the
    // structural rung is Skipped, leaving fuzzy and position as the only
    // active rungs.

    let file = dir.path().join("notes.txt");
    std::fs::write(
        &file,
        "alpha keyword one\nbeta keyword two\ngamma keyword three\ndelta keyword four\n",
    )
    .unwrap();
    let v1 = SourceFile::read(&file).unwrap();
    let bundle = SelectorBundle::capture_full(&v1, LineRange::new(1, 4).unwrap()).unwrap();
    assert!(bundle.git.is_none(), "non-repo file gets no git selector");
    assert!(
        bundle.structural.is_none(),
        "unparsed `.txt` gets no structural selector",
    );

    // Insert one prefix line and edit one line in place: fuzzy will hit
    // 2:5 (a one-line shift), but quote/structural/git have no signal here.
    std::fs::write(
        &file,
        "// prefix\nalpha keyword one\nbeta keyword TWO\ngamma keyword three\ndelta keyword four\n",
    )
    .unwrap();
    let v2 = SourceFile::read(&file).unwrap();

    let res = resolve(&bundle, &v2, None);

    assert!(
        matches!(res.status, AnchorStatus::Orphaned { .. }),
        "fuzzy moved off the saved range with no independent witness: must \
         orphan, not silently drift onto a guess. Got {:?}",
        res.status
    );
}
