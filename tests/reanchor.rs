//! Phase 4: `reanchor` refreshes a confidently-drifted note (and only then),
//! is idempotent, and never touches an orphaned or low-confidence one — the
//! safety property that keeps it from silently welding a note to wrong code.
//!
//! Also covered: a `reanchor` run against a *dirty* working tree must not
//! desync the git rung — the git selector carries its baseline in its
//! commit's own coordinates precisely so that it cannot.

use std::path::Path;
use std::process::Command;

use ynotes::{
    GitContext, LineRange, Note, ReanchorSkipReason, Rung, RungResult, Scope, SelectorBundle,
    SourceFile, Store, reanchor, resolve,
};

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
            .iter()
            .any(|n| n.id == original.id),
        "dry run must not modify the store"
    );

    // Apply: the old note is superseded by a new content-addressed id.
    let applied = reanchor(&store, false).unwrap();
    assert_eq!(applied.changed.len(), 1);
    let live = store.notes_for("lib.rs").unwrap();
    assert_eq!(live.len(), 1, "superseded, not duplicated");
    assert_ne!(live[0].id, original.id, "id reflects the refreshed bundle");
    assert_eq!(live[0].body, "hot path");
    assert_eq!(live[0].created_at, original.created_at, "provenance kept");

    // Idempotent: nothing left to do.
    let again = reanchor(&store, false).unwrap();
    assert_eq!(again.changed.len(), 0);
    assert_eq!(again.unchanged, 1);
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
    let live = store.notes_for("a.rs").unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, note.id, "untouched: same id, same anchor");
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

/// `reanchor` against a *dirty* working tree must not desync the git rung.
/// The git selector's baseline is captured in its commit's own coordinates,
/// so after a dirty reanchor the git rung still agrees with the resolved
/// range — and keeps agreeing once the pending edit is committed.
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
    // `reanchor` workflow — you run it *after* editing, tree still dirty.
    std::fs::write(
        &file,
        "// h1\n// h2\n// h3\nfn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n    let q = y;\n}\n",
    )
    .unwrap();

    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.changed.len(), 1, "the drifted note is refreshed");

    let ctx = GitContext::discover(root).expect("repo discovered");
    let refreshed = store.notes_for("s.rs").unwrap().pop().expect("one note");

    // `beta()` is now at 8:11. The git rung must agree — not report 11:14,
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
        "the git rung agrees with the resolved range — no dirty-diff skew"
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

/// A note's target replaced by a symlink to an outside file must not be
/// refreshed by `reanchor`. The save-time guard catches the obvious case
/// (saving directly against a symlink); this catches the post-save attack —
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
