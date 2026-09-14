//! Git range transport through working-tree edits and committed changes.
//! A transported deletion point must not identify unrelated code.
//! Git-dependent cases return early when Git is unavailable.

use std::path::Path;
use std::process::Command;

use ynotes::{
    AnchorStatus, GitContext, GitSelector, LineRange, Rung, RungResult, SelectorBundle, SourceFile,
    resolve,
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

/// Schema-v1 notes have no persisted capture-time occurrence count. Their Git
/// selector still identifies the immutable saved blob, so a tracked note can
/// reconstruct that evidence and gain the fixed moved-quote behaviour without
/// being re-saved first.
#[test]
fn schema_v1_note_recovers_capture_uniqueness_from_its_git_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("notes.txt");
    std::fs::write(
        &file,
        "old heading\nBEGIN UNIQUE BLOCK\nthe invariant lives here\nEND UNIQUE BLOCK\nold footer\n",
    )
    .unwrap();
    git(root, &["add", "notes.txt"]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let ctx = GitContext::discover(root).expect("repo discovered");
    let mut bundle = SelectorBundle::capture_full(
        &SourceFile::read(&file).unwrap(),
        LineRange::new(2, 4).unwrap(),
    )
    .unwrap();
    bundle.schema = 1;
    bundle.quote.occurrences_at_capture = None;

    std::fs::write(
        &file,
        "old heading\nold footer\nunrelated section\nnew lead-in\nBEGIN UNIQUE BLOCK\nthe invariant lives here\nEND UNIQUE BLOCK\nnew trailing section\n",
    )
    .unwrap();
    let resolution = resolve(&bundle, &SourceFile::read(&file).unwrap(), Some(&ctx));

    assert_eq!(resolution.range, Some(LineRange::new(5, 7).unwrap()));
    // Moved intact ⇒ anchored; the git baseline only helped locate it.
    assert!(matches!(resolution.status, AnchorStatus::Anchored));
    assert!(resolution.rungs.iter().any(|outcome| {
        outcome.rung == Rung::Quote
            && matches!(
                outcome.result,
                RungResult::Hit {
                    range,
                    score: 90
                } if range == LineRange::new(5, 7).unwrap()
            )
    }));
}

/// A region whose enclosing construct is deleted resolves `orphaned`, not
/// `drifted`, even though git happily transports the now-deleted range onto
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
    // The note covers the whole of `doomed()`, lines 5:8 of the commit.
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

/// The dangerous complement of [`a_deleted_region_orphans_even_when_git_collapses_it`]:
/// the noted region is deleted *and* a same-shaped-but-unrelated block sits
/// where git collapses it, so fuzzy latches onto that sibling. Git can only
/// transport the deleted range onto the change point with `touched: true`, the
/// low-confidence (score 70) "carried a deleted range onto its deletion point"
/// case its own comment says the resolver should distrust. That hit must NOT
/// corroborate fuzzy's moved match: with no *independent* witness the note has
/// to orphan, not silently weld onto the unrelated block. Regression test for
/// the touched-git-witness hole (a deleted region drifting onto a coincidental
/// same-shaped sibling, which `reanchor` would then make permanent).
#[test]
fn a_touched_git_transport_does_not_witness_a_moved_fuzzy_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    // `.txt` so tree-sitter does not parse it ⇒ the structural rung is Skipped,
    // isolating the git-witness branch (fuzzy + git are the only live rungs).
    let file = root.join("acl.txt");
    std::fs::write(
        &file,
        "filler line one aaaa\n\
         filler line two bbbb\n\
         filler line three cccc\n\
         grant admin access to the primary vault\n\
         deny  admin access to the primary vault\n\
         audit admin access to the primary vault\n\
         filler line seven dddd\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let ctx = GitContext::discover(root).expect("repo discovered");
    // Note covers the admin-ACL region at lines 4:6.
    let committed = SourceFile::read(&file).unwrap();
    let bundle = SelectorBundle::capture_full(&committed, LineRange::new(4, 6).unwrap()).unwrap();
    assert!(bundle.git.is_some(), "tracked file gets a git selector");
    assert!(
        bundle.structural.is_none(),
        "unparsed `.txt` gets no structural selector",
    );

    // Replace the whole file: the admin-ACL region is gone, and a same-shaped
    // but different block (guest/backup, not admin/primary) now sits at 1:3,
    // with unrelated filler below. Commit, so git has a real replacement hunk.
    std::fs::write(
        &file,
        "grant guest access to the backup vault\n\
         deny  guest access to the backup vault\n\
         audit guest access to the backup vault\n\
         totally unrelated filler eeee\n\
         totally unrelated filler ffff\n\
         totally unrelated filler gggg\n",
    )
    .unwrap();
    git(root, &["add", "acl.txt"]);
    git(root, &["commit", "-q", "-m", "replace acl block"]);

    let after = SourceFile::read(&file).unwrap();
    let res = resolve(&bundle, &after, Some(&ctx));

    // quote misses (tokens changed); structural is Skipped; fuzzy hits the
    // guest block at 1:3, moved from the saved 4:6, and git can only
    // transport 4:6 onto the change point with `touched: true` (score 70). That
    // low-confidence hit is not evidence the region survived, so it must not
    // witness fuzzy's move: the note orphans rather than welding onto guest/backup.
    assert!(
        matches!(res.status, AnchorStatus::Orphaned { .. }),
        "a deleted region with only a touched-git witness must orphan, not \
         drift onto a same-shaped sibling: {:?} at {:?}",
        res.status,
        res.range,
    );
    assert_eq!(res.range, None, "an orphan has no resolved range");
}

/// An untracked file, one never `git add`-ed, absent from HEAD, gets no git
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
    // discoverable, but `u.rs` itself is never `git add`-ed.
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
/// provides exactly that, it carries the original range through the diff to
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
    // hits at 2:6, but that is *moved* from the saved 1:5, so the rule
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
/// location; nothing else does. Under the tightened rule that must orphan,
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

/// A note record is committed to the repository and shared, so its Git
/// selector is attacker-reachable: landing one is a pull request away. `--`
/// separates revisions from paths and cannot protect the revision slot, so a
/// `commit` of `--output=<path>` would reach `git diff` as an option and
/// overwrite that path with the diff. Every git call checks the commit is a
/// real object id first, and a rejected one degrades to "R1 contributed
/// nothing" rather than erroring.
#[test]
fn a_commit_shaped_like_an_option_never_reaches_git_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !git(root, &["init", "-q"]) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);

    let file = root.join("lib.rs");
    std::fs::write(&file, "fn one() {}\nfn target() {\n    body();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "initial"]) {
        eprintln!("skipping: git commit unavailable in this environment");
        return;
    }

    let ctx = GitContext::discover(root).expect("repo discovered");
    let rel = ctx.relativize(&file).expect("relative path");
    let loot = dir.path().join("loot.txt");

    let bundle = SelectorBundle::capture(
        &SourceFile::read(&file).unwrap(),
        LineRange::new(2, 4).unwrap(),
    )
    .unwrap()
    .with_git(GitSelector {
        commit: format!("--output={}", loot.display()),
        path: rel,
        range: Some(LineRange::new(2, 4).unwrap()),
    });

    // Edit the file so the diff git would be asked for is non-empty, without
    // the guard this is the content that lands in `loot.txt`.
    std::fs::write(&file, "fn one() {}\nfn target() {\n    changed();\n}\n").unwrap();
    let resolution = resolve(&bundle, &SourceFile::read(&file).unwrap(), Some(&ctx));

    assert!(
        !loot.exists(),
        "a crafted commit id wrote to an arbitrary path via `git diff --output`"
    );
    assert!(
        matches!(
            resolution
                .rungs
                .iter()
                .find(|o| o.rung == Rung::Git)
                .map(|o| &o.result),
            None | Some(RungResult::Skipped | RungResult::Miss)
        ),
        "R1 must contribute nothing for an unusable commit, got {:?}",
        resolution.rungs
    );
}
