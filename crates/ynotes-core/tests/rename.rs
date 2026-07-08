//! Following file renames end to end: the git rename primitive, the
//! `reanchor` re-target write path, and the `query` reverse-detection read
//! path. A file rename must not orphan a note or hide it at the new path (the
//! R1 rename gap).
//!
//! Skips itself cleanly when no usable `git` binary is present, so the suite
//! stays green on a machine without git while still covering the paths that
//! need it.

use std::path::Path;
use std::process::Command;

use ynotes::{
    GitContext, LineRange, LineSpec, Note, ReanchorSkipReason, Scope, SelectorBundle, SourceFile,
    Store, query, reanchor,
};

/// Runs a git command in `dir`, returning whether it succeeded. Used only for
/// harness setup (creating the repo under test), never as an assertion.
fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Saves a `Range` note for `range` of the file at `abs` (which must exist).
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

/// Initialises a throwaway git repo with an identity configured, or returns
/// `false` if git is unavailable in this environment (so the caller can skip).
fn init_repo(root: &Path) -> bool {
    if !git(root, &["init", "-q"]) {
        return false;
    }
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "Test"]);
    true
}

#[test]
fn renamed_to_detects_a_committed_rename() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    std::fs::write(root.join("old.rs"), "fn alpha() {\n    let x = 1;\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let ctx = GitContext::discover(root).expect("repo discovered");
    let base = ctx.head_commit().expect("HEAD commit");

    // Rename and commit.
    git(root, &["mv", "old.rs", "new.rs"]);
    git(root, &["commit", "-q", "-m", "rename"]);

    assert_eq!(
        ctx.renamed_to(&base, "old.rs").as_deref(),
        Some("new.rs"),
        "the committed rename old.rs -> new.rs must be detected"
    );
    assert_eq!(
        ctx.renamed_to(&base, "never-existed.rs"),
        None,
        "a path that was never renamed must not report a destination"
    );
}

#[test]
fn renamed_to_detects_a_staged_uncommitted_rename() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    std::fs::write(root.join("old.rs"), "fn alpha() {\n    let x = 1;\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let ctx = GitContext::discover(root).expect("repo discovered");
    let base = ctx.head_commit().expect("HEAD commit");

    // Stage the rename but do not commit it.
    git(root, &["mv", "old.rs", "new.rs"]);

    assert_eq!(
        ctx.renamed_to(&base, "old.rs").as_deref(),
        Some("new.rs"),
        "a staged-but-uncommitted rename must be detected too"
    );
}

#[test]
fn renamed_from_traces_the_reverse_chain() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    std::fs::write(root.join("old.rs"), "fn alpha() {\n    let x = 1;\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let ctx = GitContext::discover(root).expect("repo discovered");

    git(root, &["mv", "old.rs", "new.rs"]);
    git(root, &["commit", "-q", "-m", "rename"]);

    let previous = ctx.renamed_from("new.rs");
    assert!(
        previous.iter().any(|p| p == "old.rs"),
        "new.rs must trace back to old.rs, got {previous:?}"
    );
}

#[test]
fn reanchor_re_targets_a_note_after_a_committed_rename() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(
        &old,
        "fn one() {}\nfn target() {\n    work();\n}\nfn z() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }

    let store = Store::init(root).unwrap();
    // A note on `target()` (lines 2:4), saved while the file is tracked so it
    // carries a git selector (baseline commit + old path).
    let original = save_note(&store, &old, LineRange::new(2, 4).unwrap(), "hot path");
    assert!(
        original.bundle.git.is_some(),
        "the note must carry a git selector to be relocatable"
    );

    // Commit a rename. The content is unchanged, so the region is intact at the
    // new path — reanchor must migrate the note there, not orphan it.
    git(root, &["mv", "old.rs", "new.rs"]);
    git(root, &["commit", "-q", "-m", "rename"]);

    let report = reanchor(&store, false).unwrap();
    assert_eq!(
        report.relocated.len(),
        1,
        "the renamed note is relocated, not skipped: {report:?}"
    );
    let reloc = &report.relocated[0];
    assert_eq!(reloc.from_target, "old.rs");
    assert_eq!(reloc.to_target, "new.rs");
    assert_eq!(reloc.old_id, original.id, "the old id is retired");
    assert_ne!(
        reloc.new_id, original.id,
        "the relocated note gets a new id"
    );

    // The old target is emptied; the note now lives under the new one, with its
    // provenance intact.
    assert!(
        store.notes_for("old.rs").unwrap().notes.is_empty(),
        "no note left behind at the old path"
    );
    let live = store.notes_for("new.rs").unwrap().notes;
    assert_eq!(live.len(), 1, "exactly one note, at the new path");
    assert_eq!(live[0].target, "new.rs");
    assert_eq!(live[0].body, "hot path");
    assert_eq!(live[0].created_at, original.created_at, "provenance kept");
    assert_eq!(
        live[0].bundle.git.as_ref().map(|g| g.path.as_str()),
        Some("new.rs"),
        "the git selector's path is refreshed to the new path"
    );
}

/// Ten distinct boilerplate lines, enough shared content that a rename is still
/// detected when the noted region alone is replaced.
const BOILERPLATE: &str = "fn a1() {}\nfn a2() {}\nfn a3() {}\nfn a4() {}\nfn a5() {}\nfn a6() {}\nfn a7() {}\nfn a8() {}\nfn a9() {}\nfn a10() {}\n";

#[test]
fn reanchor_does_not_false_weld_a_moved_fuzzy_match_across_a_rename() {
    // Safety, invariant #10. After a rename where the noted region was DELETED,
    // a fuzzy-similar-but-different block elsewhere in the new file must NOT be
    // welded to. The danger: the note's git selector still points at the
    // renamed-away old path, so the git rung transports onto the file's top
    // (line 1); a moved fuzzy match near the top must not treat that as an
    // independent witness and pass corroboration.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    // Region NOTED_alpha at lines 11:13, below ten boilerplate lines.
    let old = root.join("old.rs");
    std::fs::write(
        &old,
        format!("{BOILERPLATE}fn NOTED_alpha() {{\n    do_alpha_thing();\n}}\n"),
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(
        &store,
        &old,
        LineRange::new(11, 13).unwrap(),
        "alpha region",
    );

    // Rename (boilerplate preserved so git still detects it), delete the alpha
    // region, and place a similar-but-different block (NOTED_beta) at the TOP,
    // where the stale git rung's line-1 transport would overlap it.
    git(root, &["mv", "old.rs", "new.rs"]);
    std::fs::write(
        root.join("new.rs"),
        format!("fn NOTED_beta() {{\n    do_beta_thing();\n}}\n{BOILERPLATE}"),
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "rename and replace region"]);

    let report = reanchor(&store, false).unwrap();
    assert!(
        report.relocated.is_empty(),
        "the deleted region must not be welded onto the different block: {report:?}"
    );
    assert_eq!(
        report.skipped.len(),
        1,
        "the note is left orphaned, not relocated"
    );
    assert_eq!(report.skipped[0].reason, ReanchorSkipReason::Orphaned);
    let live = store.notes_for("old.rs").unwrap().notes;
    assert_eq!(live[0].id, original.id, "untouched: same id, same place");

    // The query self-heal must also refuse to surface it at the new path.
    let result = query(&store, &root.join("new.rs"), LineSpec::Whole).unwrap();
    assert!(
        result.matched.is_empty(),
        "reverse detection must not false-weld either: {result:?}"
    );
}

#[test]
fn rename_following_does_not_accept_a_fuzzy_only_same_position_replacement() {
    // Safety, invariant #10. The rename-specific resolver must not treat
    // fuzzy's "same saved range" exception as enough evidence for a cross-file
    // migration: after a rename, unrelated-but-similar code can replace the
    // noted region at the same lines.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.txt");
    std::fs::write(
        &old,
        format!(
            "{BOILERPLATE}\
         grant alpha access to primary vault\n\
         deny alpha access to primary vault\n\
         audit alpha access to primary vault\n\
         keep after\n\
         keep more\n"
        ),
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(
        &store,
        &old,
        LineRange::new(11, 13).unwrap(),
        "alpha policy",
    );

    git(root, &["mv", "old.txt", "new.txt"]);
    std::fs::write(
        root.join("new.txt"),
        format!(
            "{BOILERPLATE}\
         grant beta access to primary vault\n\
         deny beta access to primary vault\n\
         audit beta access to primary vault\n\
         keep after\n\
         keep more\n"
        ),
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "rename and replace policy"]);

    let queried = query(&store, &root.join("new.txt"), LineSpec::Whole).unwrap();
    assert!(
        queried.matched.is_empty(),
        "query must not surface the alpha note on the beta replacement: {queried:?}"
    );

    let report = reanchor(&store, false).unwrap();
    assert!(
        report.relocated.is_empty(),
        "reanchor must not make the fuzzy-only replacement permanent: {report:?}"
    );
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, ReanchorSkipReason::Orphaned);
    let live = store.notes_for("old.txt").unwrap().notes;
    assert_eq!(live[0].id, original.id, "the original note stays put");
}

#[test]
fn reanchor_relocates_a_note_renamed_and_edited_in_one_commit() {
    // A small file renamed AND edited in the same commit scores below git's
    // default 50% rename similarity, so it must be detected at a lower
    // threshold — the region is still recoverable at the new path.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(
        &old,
        "fn a() {}\nfn target() {\n    let value = compute();\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(
        &store,
        &old,
        LineRange::new(3, 3).unwrap(),
        "the compute line",
    );

    // Rename and edit the noted line in the same commit.
    git(root, &["mv", "old.rs", "new.rs"]);
    std::fs::write(
        root.join("new.rs"),
        "fn a() {}\nfn target() {\n    let value = compute_v2();\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "rename and edit"]);

    let report = reanchor(&store, false).unwrap();
    assert_eq!(
        report.relocated.len(),
        1,
        "a rename+edit below git's default threshold must still relocate: {report:?}"
    );
    assert_eq!(report.relocated[0].to_target, "new.rs");
    let live = store.notes_for("new.rs").unwrap().notes;
    assert_eq!(live.len(), 1);
    assert_ne!(live[0].id, original.id);
}

#[test]
fn query_surfaces_pre_rename_notes_even_when_new_path_has_own_notes() {
    // A destination-local note must not suppress reverse rename detection:
    // otherwise pre-rename notes are hidden until a maintenance pass runs.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(&old, "fn header() {}\nfn target() {\n    work();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    save_note(
        &store,
        &old,
        LineRange::new(2, 4).unwrap(),
        "old target note",
    );

    git(root, &["mv", "old.rs", "new.rs"]);
    git(root, &["commit", "-q", "-m", "rename"]);
    save_note(
        &store,
        &root.join("new.rs"),
        LineRange::new(1, 1).unwrap(),
        "new header note",
    );

    let result = query(&store, &root.join("new.rs"), LineSpec::Whole).unwrap();
    let bodies: Vec<&str> = result
        .matched
        .iter()
        .map(|rn| rn.note.body.as_str())
        .collect();
    assert!(
        bodies.contains(&"old target note") && bodies.contains(&"new header note"),
        "query should include both direct and reverse-detected notes: {result:?}"
    );
    let old = result
        .matched
        .iter()
        .find(|rn| rn.note.body == "old target note")
        .expect("old note surfaced");
    assert_eq!(old.relocated_from.as_deref(), Some("old.rs"));
}

#[test]
fn query_surfaces_a_note_across_a_staged_but_uncommitted_rename() {
    // The common workflow: `git mv` stages the rename, the agent keeps working
    // and queries the new path before committing or reanchoring. git's *history*
    // cannot see the staged rename, so the reverse detection must also consult
    // the working-tree diff.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(&old, "fn a() {}\nfn target() {\n    work();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    save_note(&store, &old, LineRange::new(2, 3).unwrap(), "staged note");

    // Stage the rename but do NOT commit it.
    git(root, &["mv", "old.rs", "new.rs"]);

    let new = root.join("new.rs");
    let result = query(&store, &new, LineSpec::Line(2)).unwrap();
    assert_eq!(
        result.matched.len(),
        1,
        "a staged (uncommitted) rename must still surface the note at the new path"
    );
    assert_eq!(result.matched[0].relocated_from.as_deref(), Some("old.rs"));
}

#[cfg(unix)]
#[test]
fn rename_following_handles_c_quoted_pathnames() {
    // POSIX allows quotes and backslashes in filenames. Git C-quotes those in
    // normal name-status output, so the rename parser must consume the NUL
    // form instead of splitting the quoted text.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let cases = [
        (
            "old\"quote.rs",
            "new\"quote.rs",
            "quote_target",
            "quoted path note",
        ),
        (
            "old\\slash.rs",
            "new\\slash.rs",
            "slash_target",
            "backslash path note",
        ),
    ];
    for (old, _, symbol, _) in cases {
        std::fs::write(
            root.join(old),
            format!("fn keep() {{}}\nfn {symbol}() {{\n    work();\n}}\n"),
        )
        .unwrap();
        git(root, &["add", old]);
    }
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    for (old, _, _, body) in cases {
        save_note(&store, &root.join(old), LineRange::new(2, 4).unwrap(), body);
    }

    for (old, new, _, _) in cases {
        git(root, &["mv", old, new]);
    }
    git(root, &["commit", "-q", "-m", "rename special paths"]);

    for (_, new, _, body) in cases {
        let result = query(&store, &root.join(new), LineSpec::Whole).unwrap();
        assert_eq!(
            result.matched.len(),
            1,
            "query should follow rename for {new}: {result:?}"
        );
        assert_eq!(result.matched[0].note.body, body);
    }

    let report = reanchor(&store, false).unwrap();
    assert_eq!(
        report.relocated.len(),
        2,
        "both special-path notes should relocate: {report:?}"
    );
}

#[test]
fn query_surfaces_a_note_renamed_and_edited_in_one_commit() {
    // The query-side twin of the rename+edit reanchor case: a small file
    // renamed and edited in one commit must still surface at the new path.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(
        &old,
        "fn a() {}\nfn target() {\n    let value = compute();\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    save_note(&store, &old, LineRange::new(3, 3).unwrap(), "compute line");

    git(root, &["mv", "old.rs", "new.rs"]);
    std::fs::write(
        root.join("new.rs"),
        "fn a() {}\nfn target() {\n    let value = compute_v2();\n}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "rename and edit"]);

    let new = root.join("new.rs");
    let result = query(&store, &new, LineSpec::Line(3)).unwrap();
    assert_eq!(
        result.matched.len(),
        1,
        "a rename+edit must still surface the note at the new path: {result:?}"
    );
    assert_eq!(result.matched[0].relocated_from.as_deref(), Some("old.rs"));
}

#[test]
fn reanchor_dry_run_reports_a_relocation_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(&old, "fn a() {}\nfn target() {\n    work();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(&store, &old, LineRange::new(2, 3).unwrap(), "note");
    git(root, &["mv", "old.rs", "new.rs"]);
    git(root, &["commit", "-q", "-m", "rename"]);

    let preview = reanchor(&store, true).unwrap();
    assert_eq!(preview.relocated.len(), 1, "the relocation is previewed");
    // Nothing moved on disk: the note is still filed under the old target.
    assert!(
        store
            .notes_for("old.rs")
            .unwrap()
            .notes
            .iter()
            .any(|n| n.id == original.id),
        "dry run must not move the note"
    );
    assert!(store.notes_for("new.rs").unwrap().notes.is_empty());
}

#[test]
fn reanchor_declines_to_relocate_when_the_region_was_deleted_in_the_rename() {
    // The wrong-file guard: git may detect a rename, but if the noted region is
    // gone at the new path, the note must stay orphaned — never welded onto the
    // renamed file at some unrelated line (the P2 property this protects).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    // Ten lines, mostly shared boilerplate so the file stays >50% similar after
    // the region is removed (git still calls it a rename), but the noted region
    // (2:4, a distinctive unique block) is deleted.
    std::fs::write(
        &old,
        "fn keep_a() {}\nfn DISTINCT_REGION() {\n    unique_marker_9137();\n}\nfn keep_b() {}\nfn keep_c() {}\nfn keep_d() {}\nfn keep_e() {}\nfn keep_f() {}\nfn keep_g() {}\nfn keep_h() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(&store, &old, LineRange::new(2, 4).unwrap(), "the region");

    // Rename AND delete the distinctive region, keeping the boilerplate so git
    // still detects the rename.
    git(root, &["mv", "old.rs", "new.rs"]);
    std::fs::write(
        root.join("new.rs"),
        "fn keep_a() {}\nfn keep_b() {}\nfn keep_c() {}\nfn keep_d() {}\nfn keep_e() {}\nfn keep_f() {}\nfn keep_g() {}\nfn keep_h() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "rename and drop region"]);

    let report = reanchor(&store, false).unwrap();
    assert!(
        report.relocated.is_empty(),
        "a deleted region must not relocate onto the renamed file: {report:?}"
    );
    assert_eq!(
        report.skipped.len(),
        1,
        "the note is skipped, not relocated"
    );
    assert_eq!(
        report.skipped[0].reason,
        ReanchorSkipReason::Orphaned,
        "the honest verdict is orphaned — the region's code is gone"
    );
    // The note is untouched: still under the old target, same id.
    let live = store.notes_for("old.rs").unwrap().notes;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, original.id, "untouched: same id");
}

#[test]
fn reanchor_relocates_when_the_store_is_in_a_git_subdirectory() {
    // Coordinate check: when `.ynotes` sits in a subdirectory of the git repo,
    // a note's `target` is store-relative while its git selector's path is
    // git-relative — the two differ. Relocation must translate git's
    // (git-relative) rename destination back to a store target correctly.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    let old = sub.join("old.rs");
    std::fs::write(&old, "fn a() {}\nfn target() {\n    work();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    // Store lives in the subdirectory, not at the git root.
    let store = Store::init(&sub).unwrap();
    let original = save_note(&store, &old, LineRange::new(2, 3).unwrap(), "sub note");
    assert_eq!(original.target, "old.rs", "target is store-relative");
    assert_eq!(
        original.bundle.git.as_ref().map(|g| g.path.as_str()),
        Some("sub/old.rs"),
        "git selector path is git-relative — the two coordinate systems differ"
    );

    git(root, &["mv", "sub/old.rs", "sub/new.rs"]);
    git(root, &["commit", "-q", "-m", "rename in subdir"]);

    let report = reanchor(&store, false).unwrap();
    assert_eq!(report.relocated.len(), 1, "relocated across the rename");
    assert_eq!(report.relocated[0].from_target, "old.rs");
    assert_eq!(
        report.relocated[0].to_target, "new.rs",
        "the git-relative destination is translated back to a store target"
    );
    let live = store.notes_for("new.rs").unwrap().notes;
    assert_eq!(live.len(), 1);
    assert_eq!(
        live[0].bundle.git.as_ref().map(|g| g.path.as_str()),
        Some("sub/new.rs"),
        "the refreshed git selector path is git-relative to the new file"
    );
}

#[test]
fn reanchor_defers_relocating_a_staged_rename_until_committed_so_the_note_keeps_a_git_baseline() {
    // A staged-but-uncommitted rename destination is not in HEAD, so a note
    // relocated there could carry no git selector — losing R1 and the ability to
    // follow a *later* rename. `reanchor` therefore defers: it migrates only to a
    // committed path (which yields a valid baseline). `query` self-heal still
    // serves the read in the meantime.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(&old, "fn a() {}\nfn target() {\n    work();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(&store, &old, LineRange::new(2, 3).unwrap(), "staged");

    // Stage the rename but do not commit it: reanchor must NOT migrate yet.
    git(root, &["mv", "old.rs", "new.rs"]);
    let staged = reanchor(&store, false).unwrap();
    assert!(
        staged.relocated.is_empty(),
        "a staged (uncommitted) rename is not migrated by reanchor: {staged:?}"
    );
    assert!(
        store
            .notes_for("old.rs")
            .unwrap()
            .notes
            .iter()
            .any(|n| n.id == original.id),
        "the note stays put until the rename is committed"
    );

    // Commit the rename: now reanchor migrates it, with a git baseline intact.
    git(root, &["commit", "-q", "-m", "rename"]);
    let committed = reanchor(&store, false).unwrap();
    assert_eq!(committed.relocated.len(), 1, "committed rename migrates");
    let live = store.notes_for("new.rs").unwrap().notes;
    assert_eq!(live.len(), 1);
    assert!(
        live[0].bundle.git.is_some(),
        "the migrated note carries a git selector (a valid baseline for future re-following)"
    );
}

#[test]
fn reanchor_does_not_relocate_a_note_that_was_untracked_at_save() {
    // A note saved against an untracked file carries no git selector, so there
    // is no baseline to follow a rename from — a plain `mv` is indistinguishable
    // from a deletion. It must report a genuine missing target, not relocate.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let store = Store::init(root).unwrap();
    // Write the file but never `git add` it: untracked at save time.
    let old = root.join("loose.rs");
    std::fs::write(&old, "fn a() {}\nfn target() {\n    work();\n}\n").unwrap();
    let original = save_note(&store, &old, LineRange::new(2, 3).unwrap(), "loose note");
    assert!(
        original.bundle.git.is_none(),
        "an untracked file yields no git selector"
    );

    // Move it on disk; with no git baseline the rename cannot be followed.
    std::fs::rename(&old, root.join("renamed.rs")).unwrap();

    let report = reanchor(&store, false).unwrap();
    assert!(
        report.relocated.is_empty(),
        "no git selector ⇒ no relocation"
    );
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        report.skipped[0].reason,
        ReanchorSkipReason::MissingTarget,
        "an untracked, moved note is a genuine missing target"
    );
}

#[test]
fn query_surfaces_a_note_at_the_new_path_after_a_rename_without_reanchor() {
    // The read-side self-heal: an agent working at the new path must find the
    // note immediately, before any maintenance pass has migrated it.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(
        &old,
        "fn one() {}\nfn target() {\n    work();\n}\nfn z() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    let original = save_note(&store, &old, LineRange::new(2, 4).unwrap(), "hot path");

    git(root, &["mv", "old.rs", "new.rs"]);
    git(root, &["commit", "-q", "-m", "rename"]);

    // No reanchor: query the NEW path directly, at a line inside the region.
    let new = root.join("new.rs");
    let result = query(&store, &new, LineSpec::Line(3)).unwrap();
    assert_eq!(
        result.matched.len(),
        1,
        "the note is surfaced at the new path via reverse rename detection"
    );
    let rn = &result.matched[0];
    assert_eq!(rn.note.body, "hot path");
    assert_eq!(
        rn.relocated_from.as_deref(),
        Some("old.rs"),
        "the note is flagged as surfaced from its pre-rename path"
    );
    assert_eq!(
        rn.note.id, original.id,
        "the stored note is unchanged — query is read-only (invariant #8)"
    );
    // The store still files it under the old target (nothing was written).
    assert!(store.notes_for("new.rs").unwrap().notes.is_empty());
    assert!(!store.notes_for("old.rs").unwrap().notes.is_empty());
}

#[test]
fn query_at_the_new_path_does_not_surface_a_note_whose_region_was_deleted() {
    // Read-side wrong-file guard: reverse detection must not surface a note
    // whose region is gone at the queried file — an orphan does not belong to a
    // file it cannot resolve in.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let old = root.join("old.rs");
    std::fs::write(
        &old,
        "fn keep_a() {}\nfn DISTINCT_REGION() {\n    unique_marker_9137();\n}\nfn keep_b() {}\nfn keep_c() {}\nfn keep_d() {}\nfn keep_e() {}\nfn keep_f() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    save_note(&store, &old, LineRange::new(2, 4).unwrap(), "the region");

    git(root, &["mv", "old.rs", "new.rs"]);
    std::fs::write(
        root.join("new.rs"),
        "fn keep_a() {}\nfn keep_b() {}\nfn keep_c() {}\nfn keep_d() {}\nfn keep_e() {}\nfn keep_f() {}\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "rename and drop region"]);

    let new = root.join("new.rs");
    let result = query(&store, &new, LineSpec::Whole).unwrap();
    assert!(
        result.matched.is_empty() && result.orphaned.is_empty(),
        "a deleted region must not be surfaced at the renamed file: {result:?}"
    );
}

#[test]
fn query_does_not_surface_a_deleted_files_note_on_an_unrelated_file() {
    // No-false-positive guard for the lowered rename threshold: a file that was
    // genuinely *deleted* (not renamed) must not have its note surface on some
    // other, unrelated file the agent happens to query. git must not pair them
    // as a rename, and even if it did, corroboration would decline.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    std::fs::write(root.join("deleted.rs"), "fn gone() {\n    x();\n}\n").unwrap();
    std::fs::write(root.join("kept.rs"), "fn other() {\n    y();\n}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();
    save_note(
        &store,
        &root.join("deleted.rs"),
        LineRange::new(1, 3).unwrap(),
        "doomed",
    );

    // Genuinely delete the file (no rename).
    git(root, &["rm", "-q", "deleted.rs"]);
    git(root, &["commit", "-q", "-m", "delete"]);

    let result = query(&store, &root.join("kept.rs"), LineSpec::Whole).unwrap();
    assert!(
        result.matched.is_empty() && result.orphaned.is_empty(),
        "a deleted file's note must not surface on an unrelated file: {result:?}"
    );
}

#[test]
fn query_at_a_note_less_file_with_no_rename_history_stays_empty() {
    // Reverse detection must not invent notes for an ordinary file that simply
    // has none — no false positives, and no cost beyond the one git call.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    let file = root.join("fresh.rs");
    std::fs::write(&file, "fn a() {}\nfn b() {}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let store = Store::init(root).unwrap();

    let result = query(&store, &file, LineSpec::Whole).unwrap();
    assert!(
        result.matched.is_empty() && result.orphaned.is_empty(),
        "a file with no notes and no rename history yields nothing"
    );
}

#[test]
fn renamed_to_is_silent_when_the_path_still_exists() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !init_repo(root) {
        eprintln!("skipping: no usable `git` binary");
        return;
    }
    std::fs::write(root.join("kept.rs"), "fn alpha() {}\n").unwrap();
    git(root, &["add", "."]);
    if !git(root, &["commit", "-q", "-m", "init"]) {
        eprintln!("skipping: git commit unavailable");
        return;
    }
    let ctx = GitContext::discover(root).expect("repo discovered");
    let base = ctx.head_commit().expect("HEAD commit");

    // No rename at all: the file is still where it was.
    assert_eq!(
        ctx.renamed_to(&base, "kept.rs"),
        None,
        "an unrenamed, still-present path must report no destination"
    );
}
