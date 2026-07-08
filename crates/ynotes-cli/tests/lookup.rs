//! `lookup`: resolve a stable `(target, body)` handle to the live note(s),
//! since the content-addressed id rotates on reanchor/update.

use std::path::Path;

use assert_cmd::Command;
use ynotes::{LineRange, Note, Scope, SelectorBundle, SourceFile, Store, lookup};

fn yn() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the binary")
}

/// Saves a Range note for `range` of the file at `abs` (mirrors the helper in
/// tests/reanchor.rs).
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
fn lookup_filters_by_target_and_body_substring() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    std::fs::write(&a, "one\ntwo\nthree\nfour\n").unwrap();
    std::fs::write(&b, "alpha\nbeta\n").unwrap();
    save_note(
        &store,
        &a,
        LineRange::new(1, 1).unwrap(),
        "token refresh logic",
    );
    save_note(&store, &a, LineRange::new(3, 3).unwrap(), "unrelated note");
    save_note(
        &store,
        &b,
        LineRange::new(1, 1).unwrap(),
        "token refresh elsewhere",
    );

    // target only: both notes for a.rs.
    let by_target = lookup(&store, Some(&a), None).unwrap().notes;
    assert_eq!(by_target.len(), 2);

    // body only (no target): both "token refresh" notes across files.
    let by_body = lookup(&store, None, Some("token refresh")).unwrap().notes;
    assert_eq!(by_body.len(), 2);
    assert!(
        by_body
            .iter()
            .all(|rn| rn.note.body.contains("token refresh"))
    );

    // both: just the a.rs token-refresh note.
    let both = lookup(&store, Some(&a), Some("token refresh"))
        .unwrap()
        .notes;
    assert_eq!(both.len(), 1);
    assert_eq!(both[0].note.target, "a.rs");

    // case-sensitive: no match for a different case.
    let none = lookup(&store, None, Some("TOKEN REFRESH")).unwrap().notes;
    assert!(none.is_empty(), "body match is case-sensitive");
}

#[test]
fn lookup_requires_a_filter() {
    let dir = tempfile::tempdir().unwrap();
    yn().current_dir(dir.path()).arg("init").assert().success();
    yn().current_dir(dir.path())
        .arg("lookup")
        .assert()
        .failure()
        .code(2);
}

#[test]
fn lookup_finds_by_body_and_exits_zero_on_no_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.rs"), "a\nb\nc\n").unwrap();
    yn().current_dir(dir.path()).arg("init").assert().success();
    yn().current_dir(dir.path())
        .args(["save", "f.rs", "2", "-m", "find this body"])
        .assert()
        .success();

    // Match: the id is printed.
    let out = yn()
        .current_dir(dir.path())
        .args(["lookup", "--body-contains", "find this", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["data"]["notes"].as_array().unwrap().len(), 1);

    // No match: still exit 0 (it is a query, not an error).
    yn().current_dir(dir.path())
        .args(["lookup", "--body-contains", "no such text"])
        .assert()
        .success();
}
