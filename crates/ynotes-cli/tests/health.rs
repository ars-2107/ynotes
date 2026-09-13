//! `doctor` store-health diagnostics (the `health` object on `doctor --json`):
//! malformed records, index drift, and missing managed `.gitattributes` lines.
//! Surfaced read-only, `doctor` writes nothing it reports on.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

fn ynotes() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the binary")
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

fn doctor_health(workdir: &Path) -> serde_json::Value {
    let out = ynotes()
        .current_dir(workdir)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    v["data"]["health"].clone()
}

#[test]
fn doctor_health_is_clean_on_a_healthy_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "a.rs", "1", "-m", "x"])
        .assert()
        .success();

    let health = doctor_health(dir.path());
    assert_eq!(health["malformed"].as_array().unwrap().len(), 0);
    assert_eq!(health["index"]["status"], "healthy");
    assert_eq!(health["index"]["recovered"], serde_json::json!(0));
    assert_eq!(health["index"]["dangling"], serde_json::json!(0));
    assert_eq!(health["gitattributes_missing"].as_array().unwrap().len(), 0);
}

#[test]
fn doctor_health_flags_a_malformed_record_and_missing_gitattributes() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "a.rs", "1", "-m", "x"])
        .assert()
        .success();

    // Corrupt the one record, and strip the managed merge guards (as a store
    // created before the guard existed would lack them).
    std::fs::write(first_note_file(dir.path()), "garbage").unwrap();
    std::fs::write(dir.path().join(".ynotes/.gitattributes"), "").unwrap();

    let health = doctor_health(dir.path());
    assert_eq!(health["index"]["status"], "healthy");
    assert_eq!(
        health["malformed"].as_array().unwrap().len(),
        1,
        "the corrupt record is flagged"
    );
    assert!(
        !health["gitattributes_missing"]
            .as_array()
            .unwrap()
            .is_empty(),
        "the missing merge guards are flagged"
    );
}

/// The human `doctor` form must point a corrupt record at the verb that
/// actually clears it (`ynotes delete <id>`), not just list its path. This is
/// the advisory `write_health_text` prints when `malformed > 0`.
#[test]
fn doctor_text_form_points_a_malformed_record_at_delete() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "a.rs", "1", "-m", "x"])
        .assert()
        .success();
    std::fs::write(first_note_file(dir.path()), "garbage").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .arg("doctor")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("unreadable"),
        "doctor flags the corrupt record: {stdout}"
    );
    assert!(
        stdout.contains("ynotes delete"),
        "and points at the verb that clears it: {stdout}"
    );
}

#[test]
fn doctor_health_counts_a_corrupt_note_as_malformed_not_dangling() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "a.rs", "1", "-m", "x"])
        .assert()
        .success();

    // Corrupt the record's *contents*, the file still exists on disk, the
    // index still points at it.
    std::fs::write(first_note_file(dir.path()), "garbage").unwrap();

    let health = doctor_health(dir.path());
    // The bad record belongs in exactly one bucket: `malformed`. It must NOT
    // also inflate `dangling`, whose meaning is "an index pointer with no file
    // behind it at all", here the file is present, just unreadable.
    assert_eq!(health["malformed"].as_array().unwrap().len(), 1);
    assert_eq!(
        health["index"]["dangling"],
        serde_json::json!(0),
        "a present-but-corrupt record is malformed, not dangling"
    );
    assert_eq!(health["index"]["recovered"], serde_json::json!(0));
}

#[test]
fn doctor_health_reports_index_drift_when_the_index_is_deleted() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "a.rs", "1", "-m", "x"])
        .assert()
        .success();

    // Delete the cache; the note on disk survives. doctor must read it as
    // drift (a recoverable pointer), not silently as an empty store.
    std::fs::remove_file(dir.path().join(".ynotes/index/by-path.json")).unwrap();

    let health = doctor_health(dir.path());
    assert_eq!(health["index"]["status"], "missing");
    assert_eq!(
        health["index"]["recovered"],
        serde_json::json!(1),
        "the surviving note shows as a recoverable pointer"
    );
}

/// `doctorData.store` has three documented shapes: `"none"`, `{found}`, and
/// `{unreadable}`. The third had no test, so nothing stopped a refactor from
/// collapsing an unreadable store into "none found", which would tell a user
/// their repository has no notes when in fact it has notes it cannot read.
/// That is the worst possible answer from the diagnostic command.
#[test]
fn doctor_reports_a_located_but_unreadable_store_as_unreadable_not_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn target() {\n    work();\n}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1:3", "-m", "load-bearing"])
        .assert()
        .success();

    // Replace `notes/` with a regular file: the store is still located (its
    // marker is intact) but enumerating records now fails. Portable, and it
    // does not depend on running as an unprivileged user the way a chmod would.
    let notes = dir.path().join(".ynotes/notes");
    std::fs::remove_dir_all(&notes).expect("remove notes dir");
    std::fs::write(&notes, "not a directory").expect("write file in its place");

    let out = ynotes()
        .current_dir(dir.path())
        .args(["doctor", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let store = &v["data"]["store"];

    assert!(
        store.get("unreadable").is_some(),
        "a store that exists but cannot be read must report `unreadable`, got {store}"
    );
    assert_ne!(
        store.as_str(),
        Some("none"),
        "collapsing an unreadable store into \"none\" would report no notes \
         for a repository that has them"
    );
    let unreadable = &store["unreadable"];
    assert!(
        unreadable
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|r| !r.is_empty()),
        "the reason must say what went wrong, got {unreadable}"
    );

    // The text form must not disagree with the JSON form.
    let text = ynotes()
        .current_dir(dir.path())
        .arg("doctor")
        .assert()
        .success();
    let text = String::from_utf8_lossy(&text.get_output().stdout).to_lowercase();
    assert!(
        text.contains("unreadable"),
        "the human form must flag it too, got: {text}"
    );
    assert!(
        !text.contains("none found"),
        "the human form must not call an unreadable store missing, got: {text}"
    );
}
