//! Locks the machine-facing contract an AI coding agent depends on: the
//! `--json` schema and the content-addressed, deterministic note id. If this
//! snapshot changes, the agent contract changed — that must be deliberate
//! (and a `v` bump), never accidental.

use assert_cmd::Command;

fn ynotes() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the binary")
}

#[test]
fn json_contract_is_stable_and_id_is_deterministic() {
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

    // save --json yields a capturable identity record; the id is a pure
    // content hash, so it is identical on every machine.
    let save_out = ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "2", "-m", "load-bearing", "--json"])
        .assert()
        .success();
    let save_json = String::from_utf8(save_out.get_output().stdout.clone()).unwrap();
    // The id is content-addressed over (target, scope, bundle, body) and the
    // *internal* `Scope::Line` enum is preserved (invariant #6) — every
    // JSON contract bump has been presentation-only (v3 collapsed `Line`
    // and `Range` to a single `"scope":"range"`; v4 wrapped payloads in
    // the `{success, v, data}` envelope; v5 unified `query`'s
    // `matched`/`orphaned` split into a single `notes` array matching
    // `list`). So this id hash is the same as v2; only the surface
    // representation moved.
    insta::assert_snapshot!(save_json.trim(), @r#"
    {
      "success": true,
      "v": 5,
      "data": {
        "id": "23df0ca3934b6b67a004dbaa21c5daf668869bee013d55f5b6f7768fabfd5ae0",
        "target": "code.rs",
        "scope": "range",
        "range": [
          2,
          2
        ],
        "created": true
      }
    }
    "#);

    let query_out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "2", "--json"])
        .assert()
        .success();
    let query_json = String::from_utf8(query_out.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(query_json.trim(), @r#"
    {
      "success": true,
      "v": 5,
      "data": {
        "query": {
          "file": "code.rs",
          "at": "2"
        },
        "notes": [
          {
            "id": "23df0ca3934b6b67a004dbaa21c5daf668869bee013d55f5b6f7768fabfd5ae0",
            "target": "code.rs",
            "scope": "range",
            "status": "anchored",
            "stale": false,
            "resolved_range": [
              2,
              2
            ],
            "body": "load-bearing"
          }
        ],
        "warnings": []
      }
    }
    "#);
}

/// `save --json` must emit *valid* JSON even when the target path contains a
/// character JSON has to escape. It is built with `serde_json`, not string
/// interpolation, so this holds. Unix-only: Windows forbids `"` in a filename,
/// so the case cannot arise there.
#[cfg(unix)]
#[test]
fn save_json_escapes_a_special_character_in_the_target_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    // A double quote is legal in a POSIX filename and must be escaped in JSON.
    let name = "we\"ird.rs";
    std::fs::write(dir.path().join(name), "fn a() {}\n").unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["save", name, "1", "-m", "body", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();

    // The output is the agent contract: it must parse, and the target must
    // round-trip through the escaping unchanged.
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("save --json must emit valid JSON");
    assert_eq!(parsed["success"], serde_json::json!(true));
    assert_eq!(parsed["v"], serde_json::json!(5));
    assert_eq!(parsed["data"]["target"], serde_json::json!(name));
    assert_eq!(parsed["data"]["created"], serde_json::json!(true));
}

/// `--json` always emits the envelope on stdout, even on failure — a JSON
/// consumer should never need to branch on the exit code to parse the
/// response. The `success` field is the in-band signal; the `type` field
/// lets a consumer branch on error class without regex on the message.
#[test]
fn query_json_emits_failure_envelope_on_invalid_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    // A non-UTF-8 file is invalid input for `query` (the engine needs text).
    std::fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, 0xff]).unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "bin.dat", "1", "--json"])
        .assert()
        .failure()
        .code(1);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("--json must always emit valid JSON, even on failure");
    assert_eq!(parsed["success"], serde_json::json!(false));
    assert_eq!(parsed["v"], serde_json::json!(5));
    assert_eq!(parsed["type"], serde_json::json!("engine"));
    assert!(
        parsed["error"]
            .as_str()
            .unwrap()
            .contains("not valid UTF-8"),
        "error names the cause: {:?}",
        parsed["error"]
    );
}

/// `reanchor --json` is the scripting surface for the maintenance pass. Its
/// shape (`changed[]`, `skipped[]`, `unchanged`) matches the text report but
/// is machine-stable, and `skipped[].reason_code` is a stable enum a consumer
/// can branch on without parsing prose. Snapshot the empty-store case — its
/// shape is what an agent sees on a clean repo — plus the orphan skip case
/// (no `changed`, one `skipped` with `reason_code: "orphaned"`).
#[test]
fn reanchor_json_envelope_is_stable_on_an_empty_store_and_on_an_orphan() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    // Empty store: every count is zero, every array empty. The shape is
    // exactly what a consumer must accept on a clean repo.
    let empty = ynotes()
        .current_dir(dir.path())
        .args(["reanchor", "--json"])
        .assert()
        .success();
    let empty_json = String::from_utf8(empty.get_output().stdout.clone()).unwrap();
    insta::assert_snapshot!(empty_json.trim(), @r#"
    {
      "success": true,
      "v": 5,
      "data": {
        "dry_run": false,
        "changed": [],
        "skipped": [],
        "unchanged": 0
      }
    }
    "#);

    // Save a note, then obliterate the content it anchors to. The next
    // `reanchor` cannot locate it, so it lands in `skipped` with the
    // `orphaned` reason code — never refreshed (invariant #8: re-anchoring
    // would weld it to wrong code).
    std::fs::write(
        dir.path().join("code.rs"),
        "fn unique_anchor_target_42() { let x = 1; }\n",
    )
    .unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "code.rs", "1", "-m", "anchor-bait"])
        .assert()
        .success();
    std::fs::write(
        dir.path().join("code.rs"),
        "wholly different content with nothing in common\n",
    )
    .unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["reanchor", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(parsed["success"], serde_json::json!(true));
    assert_eq!(parsed["v"], serde_json::json!(5));
    let data = &parsed["data"];
    assert_eq!(data["dry_run"], serde_json::json!(false));
    assert_eq!(data["changed"].as_array().unwrap().len(), 0);
    assert_eq!(data["unchanged"], serde_json::json!(0));
    let skipped = data["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "the bait note skipped as orphan");
    assert_eq!(skipped[0]["reason_code"], serde_json::json!("orphaned"));
    assert_eq!(skipped[0]["target"], serde_json::json!("code.rs"));
}
