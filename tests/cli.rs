//! Black-box tests for the `ynotes` binary.
//!
//! These spawn the actual compiled binary with `assert_cmd` and assert only on
//! what a user or script can observe: stdout, stderr, exit code. They never
//! reach into internal APIs (bat / jujutsu integration-test style).

use assert_cmd::Command;
use predicates::prelude::*;

/// A `Command` for the freshly built `ynotes` binary.
fn ynotes() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the `ynotes` binary")
}

#[test]
fn version_flag_prints_version_and_succeeds() {
    ynotes()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_flag_succeeds() {
    ynotes().arg("--help").assert().success();
}

#[test]
fn unknown_subcommand_is_a_usage_error() {
    // clap rejects an unknown subcommand with exit code 2 (usage), the same
    // contract `CommandError::exit_code` promises for usage failures.
    ynotes()
        .arg("definitely-not-a-real-command")
        .assert()
        .failure()
        .code(2);
}

#[test]
fn doctor_succeeds_and_reports_environment() {
    // Run in an empty dir so the store line is deterministically "none found"
    // rather than depending on where the test binary was invoked.
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("ynotes"))
        .stdout(predicate::str::contains("platform"))
        .stdout(predicate::str::contains("git"))
        .stdout(predicate::str::contains("store"));
}

#[test]
fn init_creates_a_store_and_is_idempotent_as_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");

    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("initialised empty ynotes store"));
    assert!(dir.path().join(".ynotes/HEAD").is_file());

    // A second init is a usage error (exit 2), not a runtime failure.
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn save_then_query_round_trips_through_the_binary() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    std::fs::write(
        dir.path().join("code.rs"),
        "fn a() {}\nfn target() {}\nfn b() {}\n",
    )
    .expect("write file");

    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "this fn is load-bearing"])
        .assert()
        .success()
        .stdout(predicate::str::contains("saved line note"));

    // Query the line: the note comes back.
    ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("this fn is load-bearing"));

    // JSON form is well-formed and carries the body.
    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "2", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["matched"][0]["body"], "this fn is load-bearing");
}

#[test]
fn query_with_a_bad_line_spec_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "a\nb\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["query", "x.rs", "not-a-range"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn list_and_reanchor_and_explain_work_through_the_binary() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(
        dir.path().join("code.rs"),
        "fn a() {}\nfn target() {}\nfn b() {}\n",
    )
    .unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "note here"])
        .assert()
        .success();

    // list shows the note.
    ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("note here"));

    // --explain surfaces the rung vector in JSON.
    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "2", "--json", "--explain"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert!(v["matched"][0]["rungs"].is_array());

    // reanchor --dry-run on an unchanged file changes nothing.
    ynotes()
        .current_dir(dir.path())
        .args(["reanchor", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("already current"));
}

#[test]
fn completions_bash_emits_a_script() {
    ynotes()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ynotes"));
}
