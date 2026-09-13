//! Black-box tests for `ynotes skills`, the agent-facing guidance surface.
//!
//! Spawned against the real binary, in an empty temp dir with `YNOTES_DIR`
//! removed where store isolation matters: the contract is that `skills`
//! works with no store and no git repository, pure embedded content.

use assert_cmd::Command;
use predicates::prelude::*;

/// A `Command` for the freshly built `ynotes` binary, isolated from any
/// ambient store the test environment might carry.
fn ynotes() -> Command {
    let mut cmd = Command::cargo_bin("ynotes").expect("cargo test builds the `ynotes` binary");
    cmd.env_remove("YNOTES_DIR");
    cmd
}

#[test]
fn skills_get_core_serves_frontmatter_led_markdown_without_a_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = ynotes()
        .current_dir(dir.path())
        .args(["skills", "get", "core"])
        .output()
        .expect("spawn");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    assert_eq!(
        stdout.lines().next(),
        Some("---"),
        "frontmatter included, the output is self-describing in agent context"
    );
    assert!(
        !stdout.contains('\u{1b}'),
        "stdout must carry no ANSI escapes"
    );
}

#[test]
fn skills_get_core_full_appends_the_reference() {
    let plain = ynotes()
        .args(["skills", "get", "core"])
        .output()
        .expect("spawn");
    let full = ynotes()
        .args(["skills", "get", "core", "--full"])
        .output()
        .expect("spawn");
    assert!(full.status.success());
    let plain = String::from_utf8(plain.stdout).expect("utf-8");
    let full = String::from_utf8(full.stdout).expect("utf-8");
    assert!(
        full.starts_with(&plain),
        "--full appends; it never rewrites the core content"
    );
    assert!(
        full.contains("\n## Command reference"),
        "the appendix is the deep reference"
    );
}

#[test]
fn skills_list_is_the_pinned_index() {
    let output = ynotes().args(["skills", "list"]).output().expect("spawn");
    assert!(output.status.success());
    insta::assert_snapshot!(String::from_utf8(output.stdout).expect("utf-8"));
}

#[test]
fn unknown_skill_name_is_a_usage_error_listing_the_valid_names() {
    ynotes()
        .args(["skills", "get", "everything"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("core, sharing, maintenance"));
}

#[test]
fn full_on_a_skill_without_a_full_variant_prints_nothing_and_exits_2() {
    ynotes()
        .args(["skills", "get", "sharing", "--full"])
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("only `core`"));
}
