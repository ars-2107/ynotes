//! Per-note read resilience: a single malformed note file must not sink a
//! read. `query`/`list` skip the bad record, still return the healthy notes,
//! and surface the bad one (invariant #4 — a failed note is flagged, never
//! silently dropped). Mirrors `reindex`'s long-standing posture, extended to
//! the read paths an agent actually queries.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

fn ynotes() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the binary")
}

/// The note id a `notes/<aa>/<rest>.json` path encodes (its filename stem,
/// prefixed by the fan-out directory). Lets a test name a record by id even
/// after its contents have been corrupted.
fn id_from_note_path(path: &Path) -> String {
    let rest = path.file_stem().unwrap().to_str().unwrap();
    let prefix = path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    format!("{prefix}{rest}")
}

/// The first `notes/<aa>/<rest>.json` record on disk, for deliberate corruption.
fn first_note_file(workdir: &Path) -> PathBuf {
    let notes = workdir.join(".ynotes/notes");
    for prefix in std::fs::read_dir(&notes).expect("notes dir") {
        let prefix = prefix.unwrap().path();
        if prefix.is_dir() {
            for file in std::fs::read_dir(&prefix).unwrap() {
                let path = file.unwrap().path();
                if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
                    return path;
                }
            }
        }
    }
    panic!("no note record found under {}", notes.display());
}

#[test]
fn query_survives_a_malformed_note_and_surfaces_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn a() {}\nfn b() {}\n").unwrap();

    // Two distinct notes on the same file (different bodies => different ids =>
    // two record files).
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "good-one"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "good-two"])
        .assert()
        .success();

    // Corrupt exactly one record on disk; the index still points at it.
    let bad = first_note_file(dir.path());
    std::fs::write(&bad, "not a valid note").unwrap();

    // The read must still succeed: the surviving note is returned and the bad
    // record is surfaced in `malformed`, not dropped and not a hard failure.
    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["success"], serde_json::json!(true));

    let notes = json["data"]["notes"].as_array().expect("notes array");
    assert_eq!(notes.len(), 1, "the one surviving note is still returned");

    let malformed = json["data"]["malformed"]
        .as_array()
        .expect("malformed array is always present");
    assert_eq!(
        malformed.len(),
        1,
        "the bad record is surfaced, not dropped"
    );
    let bad = &malformed[0];
    assert!(
        bad["id"].as_str().unwrap().len() == 64,
        "the malformed entry carries the record's id"
    );
    assert_eq!(bad["target"], serde_json::json!("code.rs"));
    assert!(
        !bad["error"].as_str().unwrap().is_empty(),
        "the malformed entry names why it could not be read"
    );
}

#[test]
fn list_survives_a_malformed_note_and_surfaces_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "fn b() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "a.rs", "1", "-m", "note-a"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "b.rs", "1", "-m", "note-b"])
        .assert()
        .success();

    // Corrupt exactly one of the two records on disk.
    std::fs::write(first_note_file(dir.path()), "not a valid note").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["list", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["success"], serde_json::json!(true));
    assert_eq!(
        json["data"]["notes"].as_array().unwrap().len(),
        1,
        "the surviving note is still listed"
    );
    assert_eq!(
        json["data"]["malformed"].as_array().unwrap().len(),
        1,
        "the bad record is surfaced in the inventory, not dropped"
    );
}

/// The text (human) form must flag the bad record too, not just `--json` — it
/// is the path `write_text_malformed` walks, otherwise untested.
#[test]
fn query_text_form_flags_a_malformed_note() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "good-one"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "good-two"])
        .assert()
        .success();
    std::fs::write(first_note_file(dir.path()), "not a valid note").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("unreadable"),
        "text query flags the corrupt record: {stdout}"
    );
    assert!(
        stdout.contains("reindex"),
        "and points at the remedy: {stdout}"
    );
    // The surviving note's body is still printed.
    assert!(
        stdout.contains("good-one") || stdout.contains("good-two"),
        "the healthy note is still shown: {stdout}"
    );
}

/// `lookup` matches on (target, body); a malformed record cannot match either.
/// It must be silently absent from the result — and crucially must not turn the
/// whole command into a hard failure.
#[test]
fn lookup_survives_a_malformed_note() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "good-one"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "good-two"])
        .assert()
        .success();
    std::fs::write(first_note_file(dir.path()), "not a valid note").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["lookup", "--target", "code.rs", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["success"], serde_json::json!(true));
    assert_eq!(
        json["data"]["notes"].as_array().unwrap().len(),
        1,
        "lookup returns the one matchable note and does not choke on the corrupt one"
    );
}

/// `reanchor` walks every note; a malformed record (which cannot be parsed into
/// a note, so cannot be anchored) must not abort the pass — the healthy notes
/// still get processed.
#[test]
fn reanchor_survives_a_malformed_note() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "good-one"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "good-two"])
        .assert()
        .success();
    std::fs::write(first_note_file(dir.path()), "not a valid note").unwrap();

    // The pass completes (exit 0) rather than failing on the unparseable record.
    let out = ynotes()
        .current_dir(dir.path())
        .args(["reanchor", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["success"], serde_json::json!(true));
}

/// `delete <id>` of a malformed record degrades gracefully: the record cannot
/// be read to match, so it cannot be selected. `delete` reports `not_found`
/// (exit 1, `success: true` envelope) — it never crashes and never removes the
/// wrong note. This is the documented limitation that a corrupt record's remedy
/// is `reindex`/manual, not `delete`; the test locks it so it cannot regress
/// into a panic or a mis-delete.
#[test]
fn delete_of_a_malformed_note_degrades_to_not_found() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "only-note"])
        .assert()
        .success();

    // Capture the id from the path *before* corrupting the contents.
    let bad = first_note_file(dir.path());
    let id = id_from_note_path(&bad);
    std::fs::write(&bad, "garbage").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["delete", &id, "--json"])
        .assert()
        .failure()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    // A success envelope describing a partition with the id unresolved — the
    // `delete` exit-code asymmetry, applied to an unreadable record.
    assert_eq!(json["success"], serde_json::json!(true));
    assert_eq!(
        json["data"]["deleted"].as_array().unwrap().len(),
        0,
        "nothing is removed — the corrupt record was never matched"
    );
    assert_eq!(
        json["data"]["not_found"].as_array().unwrap().len(),
        1,
        "the unresolvable id is reported, not silently dropped"
    );
}
