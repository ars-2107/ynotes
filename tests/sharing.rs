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
