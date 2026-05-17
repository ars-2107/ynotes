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
fn doctor_succeeds_and_reports_platform() {
    ynotes()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("ynotes"))
        .stdout(predicate::str::contains("platform"));
}

#[test]
fn completions_bash_emits_a_script() {
    ynotes()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ynotes"));
}
