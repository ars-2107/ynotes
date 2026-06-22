//! Binds real `--json` output to the published `ynotes.schema.json`. The
//! snapshot tests in `agent_contract.rs` pin the *shape* of a few outputs; this
//! validates *every* command's actual output against the schema file itself, so
//! a hand-edit that desyncs the schema from what the binary emits fails CI here
//! rather than surprising a downstream agent. Enforces invariant #7 mechanically.

use std::path::Path;

use assert_cmd::Command;
use jsonschema::Validator;

fn ynotes() -> Command {
    Command::cargo_bin("ynotes").expect("cargo test builds the binary")
}

/// Create a store. `init` has no `--json` payload, so it is run plainly and its
/// output is not validated.
fn init(dir: &Path) {
    ynotes().current_dir(dir).arg("init").assert().success();
}

/// Compile the committed schema once. `validator_for` reads the draft from the
/// schema's `$schema` (2020-12) and resolves the all-local `$ref`s.
fn schema() -> Validator {
    let src = include_str!("../ynotes.schema.json");
    let value: serde_json::Value = serde_json::from_str(src).expect("schema is valid JSON");
    jsonschema::validator_for(&value).expect("schema compiles")
}

/// Run `ynotes <args>` in `dir` and parse stdout as JSON, *without* asserting
/// the exit code — a `--json` invocation always emits the envelope on stdout,
/// including the failure and partial-failure (`delete`) cases this must cover.
fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = ynotes()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run ynotes");
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("`{args:?}` did not emit valid JSON ({e}): {:?}", out.stdout))
}

/// Assert an output validates against the whole schema (the top-level
/// `oneOf[success, failure]`), printing every violation on failure.
fn assert_conforms(validator: &Validator, label: &str, instance: &serde_json::Value) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| format!("  - {e}"))
        .collect();
    assert!(
        errors.is_empty(),
        "`{label}` output does not conform to ynotes.schema.json:\n{}\n--- instance ---\n{}",
        errors.join("\n"),
        serde_json::to_string_pretty(instance).unwrap(),
    );
}

#[test]
fn every_command_json_payload_conforms_to_the_schema() {
    let v = schema();
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();

    init(p);
    std::fs::write(p.join("code.rs"), "fn a() {}\nfn target() {}\nfn b() {}\n").unwrap();

    let save = run_json(p, &["save", "code.rs", "2", "-m", "load-bearing", "--json"]);
    assert_conforms(&v, "save", &save);
    let id = save["data"]["id"].as_str().expect("save id").to_owned();

    assert_conforms(
        &v,
        "query",
        &run_json(p, &["query", "code.rs", "2", "--json"]),
    );
    assert_conforms(
        &v,
        "query (whole file)",
        &run_json(p, &["query", "code.rs", "--json"]),
    );
    assert_conforms(&v, "list", &run_json(p, &["list", "--json"]));
    assert_conforms(
        &v,
        "lookup",
        &run_json(p, &["lookup", "--target", "code.rs", "--json"]),
    );
    assert_conforms(&v, "reindex", &run_json(p, &["reindex", "--json"]));
    assert_conforms(
        &v,
        "reindex (dry-run)",
        &run_json(p, &["reindex", "--dry-run", "--json"]),
    );
    assert_conforms(&v, "doctor", &run_json(p, &["doctor", "--json"]));
    assert_conforms(&v, "reanchor", &run_json(p, &["reanchor", "--json"]));
    assert_conforms(
        &v,
        "reanchor (dry-run)",
        &run_json(p, &["reanchor", "--dry-run", "--json"]),
    );
    assert_conforms(&v, "prune", &run_json(p, &["prune", "--dry-run", "--json"]));
    // delete --dry-run validates the populated `deleted[]` entry shape without
    // mutating the store.
    assert_conforms(
        &v,
        "delete (dry-run)",
        &run_json(p, &["delete", &id, "--dry-run", "--json"]),
    );
    // delete of a non-existent id validates the `not_found[]` partition.
    assert_conforms(
        &v,
        "delete (not found)",
        &run_json(p, &["delete", "deadbeef", "--json"]),
    );
    // update mutates last — it re-anchors and rotates the id.
    assert_conforms(
        &v,
        "update",
        &run_json(p, &["update", &id, "-m", "revised", "--json"]),
    );
}

#[test]
fn malformed_and_health_payloads_conform_to_the_schema() {
    let v = schema();
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();

    init(p);
    std::fs::write(p.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(p.join("b.rs"), "fn b() {}\n").unwrap();
    run_json(p, &["save", "a.rs", "1", "-m", "note-a", "--json"]);
    run_json(p, &["save", "b.rs", "1", "-m", "note-b", "--json"]);

    // Corrupt one record (index still points at it) and strip the merge guards,
    // so `malformed[]` and `doctor` `health` are all populated.
    let bad = first_note_file(p);
    std::fs::write(&bad, "not a valid note").unwrap();
    std::fs::write(p.join(".ynotes/.gitattributes"), "").unwrap();

    let q = run_json(p, &["query", "a.rs", "--json"]);
    assert!(
        !q["data"]["malformed"].as_array().unwrap().is_empty()
            || q["data"]["notes"].as_array().unwrap().len() == 1,
        "the corrupt store is exercised by query"
    );
    assert_conforms(&v, "query (malformed present)", &q);
    assert_conforms(
        &v,
        "list (malformed present)",
        &run_json(p, &["list", "--json"]),
    );

    let doc = run_json(p, &["doctor", "--json"]);
    let health = &doc["data"]["health"];
    assert!(
        !health["malformed"].as_array().unwrap().is_empty(),
        "doctor health surfaces the corrupt record"
    );
    assert!(
        !health["gitattributes_missing"]
            .as_array()
            .unwrap()
            .is_empty(),
        "doctor health surfaces the missing merge guards"
    );
    assert_conforms(&v, "doctor (health populated)", &doc);
}

#[test]
fn failure_envelope_conforms_to_the_schema() {
    let v = schema();
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    init(p);
    // A non-UTF-8 file is invalid input for `query` — produces the failure
    // envelope (the other branch of the top-level `oneOf`).
    std::fs::write(p.join("bin.dat"), [0u8, 1, 2, 0xff]).unwrap();
    let fail = run_json(p, &["query", "bin.dat", "1", "--json"]);
    assert_eq!(fail["success"], serde_json::json!(false));
    assert_conforms(&v, "failure envelope", &fail);
}

/// Proves the conformance checks above are not vacuous: the compiled validator
/// must *reject* output that violates the schema. Without this, a future schema
/// edit that accidentally accepts everything (e.g. a stray `additionalProperties`
/// flip) would make every `assert_conforms` pass for the wrong reason.
#[test]
fn the_validator_rejects_non_conforming_output() {
    let v = schema();
    // A success envelope whose `data` matches none of the payload `oneOf`
    // branches (each is `additionalProperties: false` with required fields).
    let unknown_payload = serde_json::json!({
        "success": true, "v": 8, "data": { "not_a_real_payload": 1 }
    });
    assert!(
        v.iter_errors(&unknown_payload).next().is_some(),
        "validator must reject an unrecognised payload"
    );
    // A wrong contract version must fail the `schemaVersion` const.
    let wrong_version = serde_json::json!({
        "success": false, "v": 999, "error": "x", "type": "engine"
    });
    assert!(
        v.iter_errors(&wrong_version).next().is_some(),
        "validator must reject a wrong contract version"
    );
    // A query payload missing the now-required `malformed` array must fail —
    // this is exactly the drift the suite exists to catch.
    let missing_malformed = serde_json::json!({
        "success": true, "v": 8,
        "data": { "query": { "file": "x", "at": "" }, "notes": [], "warnings": [] }
    });
    assert!(
        v.iter_errors(&missing_malformed).next().is_some(),
        "validator must reject a query payload missing `malformed`"
    );
}

/// The first `notes/<aa>/<rest>.json` record on disk, for deliberate corruption.
fn first_note_file(workdir: &Path) -> std::path::PathBuf {
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
