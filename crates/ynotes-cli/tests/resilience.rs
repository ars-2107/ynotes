//! Malformed records are reported separately without hiding healthy notes.

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
fn query_recovers_an_unindexed_note_without_repairing_the_index() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn kept() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "source of truth"])
        .assert()
        .success();

    let index = dir.path().join(".ynotes/index/by-path.json");
    std::fs::write(&index, b"").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["data"]["notes"].as_array().unwrap().len(), 1);
    assert_eq!(json["data"]["notes"][0]["body"], "source of truth");
    assert_eq!(json["data"]["index"]["recovered"], 1);
    assert_eq!(json["data"]["index"]["dangling"], 0);
    assert_eq!(json["data"]["index"]["status"], "healthy");
    assert_eq!(
        std::fs::read(&index).unwrap(),
        b"",
        "a read must not repair the rebuildable index cache"
    );
}

#[test]
fn query_surfaces_an_unindexed_malformed_record_without_writing() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("code.rs"), "fn lost() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "must stay visible"])
        .assert()
        .success();

    let record = first_note_file(dir.path());
    let id = id_from_note_path(&record);
    let index = dir.path().join(".ynotes/index/by-path.json");
    std::fs::write(&index, b"").unwrap();
    std::fs::write(&record, b"malformed").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let malformed = json["data"]["malformed"].as_array().unwrap();
    assert_eq!(malformed.len(), 1, "the record is flagged, never hidden");
    assert_eq!(malformed[0]["id"], id);
    assert_eq!(malformed[0]["target"], serde_json::Value::Null);
    assert!(
        malformed[0]["path"].as_str().unwrap().starts_with("notes/"),
        "the record remains identifiable even without an index target"
    );
    assert_eq!(json["data"]["index"]["recovered"], 1);
    assert_eq!(
        std::fs::read(&index).unwrap(),
        b"",
        "surfacing corruption must remain a read-only operation"
    );
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

/// The text (human) form must flag the bad record too, not just `--json`, it
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
        stdout.contains("ynotes delete "),
        "and points at the remedy that actually clears it: {stdout}"
    );
    // The surviving note's body is still printed.
    assert!(
        stdout.contains("good-one") || stdout.contains("good-two"),
        "the healthy note is still shown: {stdout}"
    );
}

/// `lookup` matches on (target, body); a malformed record cannot match either.
/// It must be silently absent from the result, and crucially must not turn the
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
/// a note, so cannot be anchored) must not abort the pass, the healthy notes
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

/// `delete <full-id>` of a malformed record *purges* it. The record cannot be
/// read to anchor it, but its id is recoverable from its path (and is shown in
/// `query`/`doctor` output), so an exact-id delete removes the file and drops
/// the index pointer. This is the deliberate removal path for a corrupt note,
/// the one verb that can clear it. A clean removal exits `0`, and the id is
/// reported under `deleted_unreadable`, partitioned from the readable
/// `deleted[]` (an unreadable record cannot supply a target/scope/body).
#[test]
fn delete_purges_an_unreadable_note_by_exact_id() {
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
        .success(); // a clean removal, exit 0, not a partial failure
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["success"], serde_json::json!(true));
    assert_eq!(
        json["data"]["deleted"].as_array().unwrap().len(),
        0,
        "no *readable* note was matched"
    );
    assert_eq!(
        json["data"]["deleted_unreadable"].as_array().unwrap(),
        &vec![serde_json::json!(id)],
        "the corrupt record is purged by its exact id"
    );
    assert_eq!(json["data"]["not_found"].as_array().unwrap().len(), 0);

    // The file is gone from disk...
    assert!(!bad.exists(), "the corrupt record file was removed");
    // ...so a subsequent query no longer surfaces a malformed record.
    let q = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "--json"])
        .assert()
        .success();
    let qj: serde_json::Value = serde_json::from_slice(&q.get_output().stdout).unwrap();
    assert!(
        qj["data"]["malformed"].as_array().unwrap().is_empty(),
        "nothing left to surface once it is purged"
    );
}

/// A *prefix* (not the exact 64-char id) of an unreadable record must NOT purge
/// it: there is no body to preview, and a short prefix could otherwise catch a
/// healthy note. It degrades to `not_found` (exit 1), like any unmatched
/// prefix, and leaves the corrupt file untouched.
#[test]
fn delete_of_an_unreadable_note_by_prefix_is_not_found() {
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

    let bad = first_note_file(dir.path());
    let id = id_from_note_path(&bad);
    std::fs::write(&bad, "garbage").unwrap();

    let prefix: String = id.chars().take(12).collect();
    let out = ynotes()
        .current_dir(dir.path())
        .args(["delete", &prefix, "--json"])
        .assert()
        .failure()
        .code(1);
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["success"], serde_json::json!(true));
    assert_eq!(
        json["data"]["deleted_unreadable"].as_array().unwrap().len(),
        0,
        "a prefix never purges an unreadable record"
    );
    assert_eq!(
        json["data"]["not_found"].as_array().unwrap().len(),
        1,
        "the prefix is reported unresolved, not silently dropped"
    );
    assert!(bad.exists(), "the corrupt record is left untouched");
}

/// `--dry-run` reports the unreadable purge without performing it: the id
/// appears in `deleted_unreadable`, but the file is still on disk.
#[test]
fn delete_dry_run_reports_an_unreadable_purge_without_removing() {
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

    let bad = first_note_file(dir.path());
    let id = id_from_note_path(&bad);
    std::fs::write(&bad, "garbage").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["delete", &id, "--dry-run", "--json"])
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(
        json["data"]["deleted_unreadable"].as_array().unwrap(),
        &vec![serde_json::json!(id)],
    );
    assert_eq!(json["data"]["dry_run"], serde_json::json!(true));
    assert!(bad.exists(), "dry-run leaves the file in place");
}
