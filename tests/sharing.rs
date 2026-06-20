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
        ynotes().current_dir(dir.path()).arg("init").assert().success();
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
    ynotes().current_dir(dir.path()).arg("init").assert().success();
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

    let out = ynotes().current_dir(dir.path()).arg("list").assert().success();
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
    ynotes().current_dir(dir.path()).arg("init").assert().success();
    std::fs::write(
        dir.path().join(".ynotes/index/by-path.json"),
        "{ broken not json",
    )
    .unwrap();
    let out = ynotes().current_dir(dir.path()).arg("list").assert().failure().code(1);
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
    ynotes().current_dir(dir.path()).arg("init").assert().success();
    let attrs = std::fs::read_to_string(dir.path().join(".ynotes/.gitattributes")).unwrap();
    assert!(
        attrs.contains("index/by-path.json merge=union"),
        "gitattributes must set the union driver: {attrs}"
    );
    let ignore = std::fs::read_to_string(dir.path().join(".ynotes/.gitignore")).unwrap();
    assert!(ignore.contains("lock"), "gitignore must exclude the lock file: {ignore}");
}

/// Running `reindex` on a store that lacks the files (e.g. created by an older
/// ynotes) restores them.
#[test]
fn reindex_adds_missing_git_integration_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes().current_dir(dir.path()).arg("init").assert().success();
    std::fs::remove_file(dir.path().join(".ynotes/.gitattributes")).unwrap();
    ynotes().current_dir(dir.path()).arg("reindex").assert().success();
    assert!(
        dir.path().join(".ynotes/.gitattributes").exists(),
        "reindex must restore a missing .gitattributes"
    );
}

/// A user-customised git-integration file is never clobbered.
#[test]
fn reindex_does_not_clobber_a_customised_gitattributes() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes().current_dir(dir.path()).arg("init").assert().success();
    let path = dir.path().join(".ynotes/.gitattributes");
    std::fs::write(&path, "# custom\n").unwrap();
    ynotes().current_dir(dir.path()).arg("reindex").assert().success();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# custom\n",
        "reindex must not clobber a customised file"
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
    ynotes().current_dir(repo).args(["save", "foo.rs", "3", "-m", "base"]).assert().success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "base"]);

    // ours: add a note at line 1.
    git(repo, &["checkout", "-q", "-b", "ours"]);
    ynotes().current_dir(repo).args(["save", "foo.rs", "1", "-m", "ours"]).assert().success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "ours"]);

    // theirs (from base): add a note at line 2.
    git(repo, &["checkout", "-q", "-"]);
    git(repo, &["checkout", "-q", "-b", "theirs"]);
    ynotes().current_dir(repo).args(["save", "foo.rs", "2", "-m", "theirs"]).assert().success();
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-qm", "theirs"]);

    // Merge theirs into ours, driven by the committed .ynotes/.gitattributes.
    git(repo, &["checkout", "-q", "ours"]);
    let merge = std::process::Command::new("git")
        .current_dir(repo)
        .args(["merge", "-q", "theirs"])
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
    let out = ynotes().current_dir(repo).args(["reindex", "--json"]).assert().success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["data"]["indexed"], serde_json::json!(3), "all three notes indexed after merge");
    assert_eq!(v["data"]["recovered"].as_array().unwrap().len(), 0, "nothing to recover");
    assert_eq!(v["data"]["dangling"].as_array().unwrap().len(), 0, "nothing dangling");
}
