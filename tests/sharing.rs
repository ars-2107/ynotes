//! Team-sharing merge safety: deterministic, union-mergeable index cache.
//!
//! These spawn the compiled binary with `assert_cmd` and assert on observable
//! behaviour (the on-disk index, exit codes, stderr), never on engine
//! internals.

use assert_cmd::Command;

fn ynotes() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the `ynotes` binary")
}

/// Two notes saved to the same file in opposite orders must produce a
/// byte-identical index. Determinism is what keeps merge diffs minimal and
/// `merge=union` well-behaved.
#[test]
fn index_is_byte_identical_regardless_of_save_order() {
    let build = |order: &[&str]| -> Vec<u8> {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("foo.rs"), "a\nb\nc\n").unwrap();
        ynotes()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success();
        for at in order {
            ynotes()
                .current_dir(dir.path())
                .args(["save", "foo.rs", at, "-m", "ctx"])
                .assert()
                .success();
        }
        std::fs::read(dir.path().join(".ynotes/index/by-path.json")).unwrap()
    };
    assert_eq!(
        build(&["1", "2"]),
        build(&["2", "1"]),
        "the index must be deterministic regardless of save order"
    );
}

/// A store whose index is still in the legacy single-object format must keep
/// working: `read_index` accepts both formats so old committed stores migrate
/// on the next write rather than breaking.
#[test]
fn a_legacy_object_format_index_is_still_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("foo.rs"), "a\nb\nc\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let out = ynotes()
        .current_dir(dir.path())
        .args(["save", "foo.rs", "1", "-m", "ctx", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let id = v["data"]["id"].as_str().unwrap().to_string();

    // Overwrite the index in the LEGACY nested-object format.
    std::fs::write(
        dir.path().join(".ynotes/index/by-path.json"),
        format!("{{\"foo.rs\":[\"{id}\"]}}"),
    )
    .unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("foo.rs"),
        "list must find the note via a legacy-format index: {stdout}"
    );
}

/// A genuinely unparseable index must fail loudly with a recovery hint, never
/// silently return empty/partial results.
#[test]
fn a_malformed_index_errors_with_a_reindex_hint() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    // No source file needed: `list` reaches read_index (the malformed index)
    // before any note content is touched.
    std::fs::write(
        dir.path().join(".ynotes/index/by-path.json"),
        "{ broken not json",
    )
    .unwrap();
    let out = ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("reindex"),
        "a malformed index must hint at reindex: {stderr}"
    );
}

/// A freshly initialised store carries the git-integration files so a
/// committed `.ynotes/` "just works" on merge with zero per-clone setup.
#[test]
fn init_writes_git_integration_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let attrs = std::fs::read_to_string(dir.path().join(".ynotes/.gitattributes")).unwrap();
    assert!(
        attrs.contains("index/by-path.json merge=union"),
        "gitattributes must set the union driver: {attrs}"
    );
    assert!(
        attrs.contains("notes/** -merge"),
        "gitattributes must guard note files from textual merge: {attrs}"
    );
    let ignore = std::fs::read_to_string(dir.path().join(".ynotes/.gitignore")).unwrap();
    assert_eq!(
        ignore, "lock\n",
        "gitignore must be exactly the lock entry: {ignore}"
    );
}

/// Running `reindex` on a store that lacks the files (e.g. created by an older
/// ynotes) restores them.
#[test]
fn reindex_adds_missing_git_integration_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::remove_file(dir.path().join(".ynotes/.gitattributes")).unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("reindex")
        .assert()
        .success();
    assert!(
        dir.path().join(".ynotes/.gitattributes").exists(),
        "reindex must restore a missing .gitattributes"
    );
}

/// `reindex --dry-run` is a preview and must write nothing — including the
/// git-integration files. Remove `.gitattributes`, dry-run, confirm it stays
/// absent.
#[test]
fn reindex_dry_run_does_not_write_git_integration_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::remove_file(dir.path().join(".ynotes/.gitattributes")).unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["reindex", "--dry-run"])
        .assert()
        .success();
    assert!(
        !dir.path().join(".ynotes/.gitattributes").exists(),
        "reindex --dry-run must not write the git-integration files"
    );
}

/// A user who pins their own driver for a pattern keeps it (we never override a
/// pattern the user already addressed), and a store predating the note-merge
/// guard gains `notes/** -merge` on the next reindex. User content is preserved.
#[test]
fn reindex_preserves_a_custom_driver_and_ensures_the_notes_guard() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let path = dir.path().join(".ynotes/.gitattributes");
    std::fs::write(&path, "# my rules\nindex/by-path.json merge=mine\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("reindex")
        .assert()
        .success();
    let attrs = std::fs::read_to_string(&path).unwrap();
    assert!(
        attrs.contains("index/by-path.json merge=mine"),
        "a custom index driver is preserved: {attrs}"
    );
    assert!(
        !attrs.contains("index/by-path.json merge=union"),
        "we must not add a second, conflicting index line: {attrs}"
    );
    assert!(
        attrs.contains("notes/** -merge"),
        "the notes guard is ensured even on a customised file: {attrs}"
    );
    assert!(
        attrs.contains("# my rules"),
        "the user's own content is preserved: {attrs}"
    );
}

/// The case the README invites and sub-project 1 missed: two teammates reanchor
/// the *same* note on two branches, then merge. `reanchor` is delete-old +
/// add-new, which git's rename detection turns into a rename/rename conflict and
/// (without `notes/** -merge`) a *textual* merge that injects conflict markers
/// into the content-addressed note JSON, corrupting the whole store. With the
/// guard the note files stay valid JSON; the merge still conflicts (rename
/// detection is unavoidable via attributes), so we resolve by keeping both valid
/// files and let `reanchor` collapse the same-lineage duplicates to one note.
#[test]
fn a_concurrent_reanchor_merge_keeps_notes_valid_and_reanchor_dedups() {
    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }
    fn note_files(repo: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let notes = repo.join(".ynotes/notes");
        for prefix in std::fs::read_dir(&notes).expect("notes dir") {
            let p = prefix.unwrap().path();
            if p.is_dir() {
                for f in std::fs::read_dir(&p).unwrap() {
                    let f = f.unwrap().path();
                    if f.extension().and_then(|e| e.to_str()) == Some("json") {
                        out.push(f);
                    }
                }
            }
        }
        out
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "t@t"]);
    git(repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("f.rs"), "a\nb\nTARGET\nd\ne\n").unwrap();
    ynotes().current_dir(repo).arg("init").assert().success();
    ynotes()
        .current_dir(repo)
        .args(["save", "f.rs", "3", "-m", "note about TARGET"])
        .assert()
        .success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "base"]);

    let base = {
        let out = std::process::Command::new("git")
            .current_dir(repo)
            .args(["symbolic-ref", "--short", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };

    // Branch A: insert one line so TARGET moves; reanchor.
    git(repo, &["checkout", "-q", "-b", "a"]);
    std::fs::write(repo.join("f.rs"), "AAA\na\nb\nTARGET\nd\ne\n").unwrap();
    ynotes()
        .current_dir(repo)
        .arg("reanchor")
        .assert()
        .success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "a"]);

    // Branch B from base: insert two different lines; reanchor.
    git(repo, &["checkout", "-q", &base]);
    git(repo, &["checkout", "-q", "-b", "b"]);
    std::fs::write(repo.join("f.rs"), "BBB\nCCC\na\nb\nTARGET\nd\ne\n").unwrap();
    ynotes()
        .current_dir(repo)
        .arg("reanchor")
        .assert()
        .success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "b"]);

    // Merge A into B. A rename/rename conflict is EXPECTED — we do not assert
    // success; we assert the note files were not corrupted.
    git(repo, &["checkout", "-q", "b"]);
    drop(
        std::process::Command::new("git")
            .current_dir(repo)
            .args(["merge", "--no-edit", "a"])
            .output()
            .expect("git merge runs"),
    );

    for f in note_files(repo) {
        let s = std::fs::read_to_string(&f).unwrap();
        assert!(
            !s.contains("<<<<<<<") && !s.contains(">>>>>>>") && !s.contains("======="),
            "note file {f:?} was corrupted with conflict markers:\n{s}"
        );
        serde_json::from_str::<serde_json::Value>(&s)
            .unwrap_or_else(|e| panic!("note file {f:?} is not valid JSON: {e}\n{s}"));
    }

    // Resolve: settle the source to one merged state and keep both valid notes.
    std::fs::write(repo.join("f.rs"), "AAA\nBBB\nCCC\na\nb\nTARGET\nd\ne\n").unwrap();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "merged"]);

    // The store is readable after the merge (no corruption).
    ynotes().current_dir(repo).arg("list").assert().success();

    // reanchor collapses the same-lineage duplicates to a single note.
    ynotes()
        .current_dir(repo)
        .arg("reanchor")
        .assert()
        .success();
    let out = ynotes()
        .current_dir(repo)
        .args(["list", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(
        v["data"]["notes"].as_array().unwrap().len(),
        1,
        "reanchor must collapse the post-merge duplicates to one note"
    );
}

/// Run a real two-branch merge where each branch adds a note to the same file.
/// With JSONL + the auto-written `merge=union` attribute, the merged index
/// stays valid and no note is lost.
#[test]
fn a_concurrent_add_merge_stays_valid_and_loses_no_note() {
    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed");
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.email", "t@t"]);
    git(repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("foo.rs"), "a\nb\nc\n").unwrap();
    ynotes().current_dir(repo).arg("init").assert().success();

    // Base commit: one note already present (matches the verified union case).
    ynotes()
        .current_dir(repo)
        .args(["save", "foo.rs", "3", "-m", "base"])
        .assert()
        .success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "base"]);

    // Record the base branch by name so `theirs` branches from the same
    // ancestor without depending on `git checkout -` reflog magic / the
    // ambient default-branch config.
    let base_branch = {
        let out = std::process::Command::new("git")
            .current_dir(repo)
            .args(["symbolic-ref", "--short", "HEAD"])
            .output()
            .expect("git symbolic-ref runs");
        assert!(out.status.success(), "git symbolic-ref failed");
        String::from_utf8(out.stdout)
            .expect("symbolic-ref output is valid UTF-8")
            .trim()
            .to_owned()
    };

    // ours: add a note at line 1.
    git(repo, &["checkout", "-q", "-b", "ours"]);
    ynotes()
        .current_dir(repo)
        .args(["save", "foo.rs", "1", "-m", "ours"])
        .assert()
        .success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "ours"]);

    // theirs (from base): add a note at line 2.
    git(repo, &["checkout", "-q", &base_branch]);
    git(repo, &["checkout", "-q", "-b", "theirs"]);
    ynotes()
        .current_dir(repo)
        .args(["save", "foo.rs", "2", "-m", "theirs"])
        .assert()
        .success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "theirs"]);

    // Merge theirs into ours, driven by the committed .ynotes/.gitattributes.
    git(repo, &["checkout", "-q", "ours"]);
    let merge = std::process::Command::new("git")
        .current_dir(repo)
        .args(["merge", "--no-edit", "-q", "theirs"])
        .output()
        .expect("git merge runs");
    assert!(
        merge.status.success(),
        "merge should not conflict: {}",
        String::from_utf8_lossy(&merge.stderr)
    );

    // The merged index is valid JSONL.
    let idx = std::fs::read_to_string(repo.join(".ynotes/index/by-path.json")).unwrap();
    for line in idx.lines().filter(|l| !l.trim().is_empty()) {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("invalid JSONL line {line:?}: {e}"));
    }

    // All three notes survive; reindex confirms the index already agrees.
    let out = ynotes()
        .current_dir(repo)
        .args(["reindex", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(
        v["data"]["indexed"],
        serde_json::json!(3),
        "all three notes indexed after merge"
    );
    assert_eq!(
        v["data"]["recovered"].as_array().unwrap().len(),
        0,
        "nothing to recover"
    );
    assert_eq!(
        v["data"]["dangling"].as_array().unwrap().len(),
        0,
        "nothing dangling"
    );
}

/// A deleted index while notes sit on disk must NOT read as "no notes" — that is
/// indistinguishable from an empty store and looks like total data loss. It must
/// fail loudly with a reindex hint, like the malformed case.
#[test]
fn a_missing_index_with_notes_is_not_silently_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("f.rs"), "a\nb\nc\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "f.rs", "2", "-m", "still here"])
        .assert()
        .success();
    std::fs::remove_file(dir.path().join(".ynotes/index/by-path.json")).unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("reindex"),
        "a missing index with notes on disk must hint at reindex, not report empty: {stderr}"
    );
}

/// A genuinely empty store whose index file is absent stays an empty result —
/// the new IndexMissing signal must fire only when notes actually exist.
#[test]
fn an_empty_store_with_missing_index_is_still_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::remove_file(dir.path().join(".ynotes/index/by-path.json")).unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .success();
}

/// reindex must recover a missing index just as it recovers a malformed one.
#[test]
fn reindex_recovers_a_missing_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("f.rs"), "a\nb\nc\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "f.rs", "2", "-m", "recover me"])
        .assert()
        .success();
    std::fs::remove_file(dir.path().join(".ynotes/index/by-path.json")).unwrap();
    let out = ynotes()
        .current_dir(dir.path())
        .args(["reindex", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["data"]["indexed"], serde_json::json!(1));
}

/// A stray non-fan-out directory under `notes/` must not be mistaken for a note
/// record: a missing index over a store with no real notes still reads as empty.
#[test]
fn a_stray_non_fanout_dir_does_not_trigger_a_false_index_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    // A non-fan-out directory (name is not 2 chars) holding a json file.
    let stray = dir.path().join(".ynotes/notes/scratch");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::write(stray.join("foo.json"), "{}").unwrap();
    std::fs::remove_file(dir.path().join(".ynotes/index/by-path.json")).unwrap();
    ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .success();
}

/// `reindex` is the documented recovery path, so it must run even when the
/// existing index is unreadable — the notes on disk are the source of truth.
/// Both raw garbage and git conflict markers must be recoverable.
#[test]
fn reindex_recovers_a_malformed_index() {
    for corruption in [
        "this is not json at all\n",
        "<<<<<<< HEAD\n{}\n=======\n>>>>>>> x\n",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f.rs"), "a\nb\nc\n").unwrap();
        ynotes()
            .current_dir(dir.path())
            .arg("init")
            .assert()
            .success();
        ynotes()
            .current_dir(dir.path())
            .args(["save", "f.rs", "2", "-m", "keep me"])
            .assert()
            .success();
        std::fs::write(dir.path().join(".ynotes/index/by-path.json"), corruption).unwrap();

        let out = ynotes()
            .current_dir(dir.path())
            .args(["reindex", "--json"])
            .assert()
            .success();
        let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
        assert_eq!(
            v["data"]["indexed"],
            serde_json::json!(1),
            "reindex must rebuild the index from notes/ despite corruption: {corruption:?}"
        );

        // The store is queryable again.
        ynotes()
            .current_dir(dir.path())
            .arg("list")
            .assert()
            .success();
    }
}
