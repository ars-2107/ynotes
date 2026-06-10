//! Black-box tests for the `ynotes` binary.
//!
//! These spawn the actual compiled binary with `assert_cmd` and assert only on
//! what a user or script can observe: stdout, stderr, exit code. They never
//! reach into internal APIs.

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
fn doctor_json_emits_a_machine_readable_report() {
    // `doctor --json` must exit 0 and emit one valid JSON object carrying the
    // same facts as the human form. Its values (platform, git version) are
    // environment-dependent, so assert only structure — never snapshot it.
    let dir = tempfile::tempdir().expect("tempdir");
    let out = ynotes()
        .current_dir(dir.path())
        .args(["doctor", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    // Envelope: every --json output rides inside `{success, v, data}`.
    assert_eq!(parsed["success"], serde_json::json!(true));
    assert_eq!(parsed["v"], serde_json::json!(5));
    let data = &parsed["data"];
    assert!(data.get("version").is_some(), "version key present");
    assert!(data.get("platform").is_some(), "platform key present");
    // `git` is always present but may be JSON null when no git binary exists.
    assert!(data.as_object().unwrap().contains_key("git"));
    assert!(data.get("store").is_some(), "store key present");
}

#[test]
fn query_on_a_nonexistent_file_names_the_path_and_exits_zero() {
    // A mistyped path must fail loudly: querying a file that does not exist
    // names it on stderr rather than silently reading as "a real file with no
    // notes". Still exit 0 — finding nothing is not a hard error.
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["query", "typo.rs"])
        .assert()
        .success()
        .stderr(predicate::str::contains("does not exist"))
        .stderr(predicate::str::contains("typo.rs"));
}

#[test]
fn init_creates_a_store_and_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");

    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("initialised empty ynotes store"));
    assert!(dir.path().join(".ynotes/HEAD").is_file());

    // A second init succeeds (exit 0) and acknowledges the existing store.
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("already exists"));
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

    ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("this fn is load-bearing"));

    let out = ynotes()
        .current_dir(dir.path())
        .args(["query", "code.rs", "2", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["success"], serde_json::json!(true));
    assert_eq!(parsed["v"], serde_json::json!(5));
    // v=5 unified `query`'s matched/orphaned split into a single `notes`
    // array (status discriminates) — match the new shape here.
    assert_eq!(
        parsed["data"]["notes"][0]["body"],
        "this fn is load-bearing"
    );
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
fn a_bare_line_zero_is_a_usage_error_not_a_whole_file_query() {
    // Line numbers are 1-based. A range `0:5` and `save … 0` already fail; a
    // bare `query <file> 0` must fail the same way (exit 2), never silently
    // widen into a whole-file query.
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "a\nb\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["query", "x.rs", "0"])
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
    assert!(v["data"]["notes"][0]["rungs"].is_array());

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

#[test]
fn an_in_place_drift_does_not_print_a_redundant_was_clause() {
    // A note edited *in place* drifts to the very range it was saved at, so
    // the human text format must not append `was X` for an X equal to the
    // resolved range — the `was …` clause is for a region that moved.
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    // A `.txt` file: tree-sitter does not parse it, so an in-region edit
    // drifts via the fuzzy rung (no structural rescue) without moving.
    let file = dir.path().join("notes.txt");
    std::fs::write(
        &file,
        "alpha keyword one\nbeta keyword two\ngamma keyword three\n",
    )
    .unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "notes.txt", "1:3", "-m", "the three keywords"])
        .assert()
        .success();

    // Edit a line *inside* the region: it drifts in place (same range).
    std::fs::write(
        &file,
        "alpha keyword one\nbeta keyword CHANGED\ngamma keyword three\n",
    )
    .unwrap();

    ynotes()
        .current_dir(dir.path())
        .args(["query", "notes.txt", "2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("drifted"))
        .stdout(predicate::str::contains("was").not());
}

/// A symlink whose *path* lives inside the work tree but whose *target* points
/// outside (e.g. `inside.lnk -> /etc/passwd`) must be refused at save. The
/// lexical `relativize` cannot catch this — its job is to work even for files
/// that do not yet exist — so the save command canonicalises both sides and
/// rejects the escape with a usage error. Without this guard, the symlink's
/// resolved contents would be read into the selector bundle; if `.ynotes` is
/// committed, that leaks whatever the saver's filesystem put behind the link.
///
/// Unix-only: a symlink to `/etc/passwd` is meaningful only there.
#[cfg(unix)]
#[test]
fn save_refuses_a_symlink_whose_target_escapes_the_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let link = dir.path().join("escape.lnk");
    std::os::unix::fs::symlink("/etc/passwd", &link).expect("symlink");

    ynotes()
        .current_dir(dir.path())
        .args(["save", "escape.lnk", "-m", "should be refused"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("escapes the work tree"));
}

/// `list <dir>` must filter notes by prefix rather than silently reporting
/// "no notes in this store" when the path is a directory. The boundary check
/// uses a trailing `/`, so `sa/` does not collide with `saas/`.
#[test]
fn list_accepts_a_directory_and_filters_by_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    std::fs::create_dir_all(dir.path().join("saas/server/src/auth")).unwrap();
    std::fs::create_dir_all(dir.path().join("saas/server/src/util")).unwrap();
    std::fs::create_dir_all(dir.path().join("sa")).unwrap();
    std::fs::write(
        dir.path().join("saas/server/src/auth/login.js"),
        "fn login() {}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("saas/server/src/util/log.js"),
        "fn log() {}\n",
    )
    .unwrap();
    for f in [
        "saas/server/src/auth/login.js",
        "saas/server/src/util/log.js",
    ] {
        ynotes()
            .current_dir(dir.path())
            .args(["save", f, "-m", "context"])
            .assert()
            .success();
    }

    // Specific subdir: matches one note, not both.
    ynotes()
        .current_dir(dir.path())
        .args(["list", "saas/server/src/auth/"])
        .assert()
        .success()
        .stdout(predicate::str::contains("auth/login.js"))
        .stdout(predicate::str::contains("util/log.js").not());

    // Parent dir: matches both, recursively.
    ynotes()
        .current_dir(dir.path())
        .args(["list", "saas/"])
        .assert()
        .success()
        .stdout(predicate::str::contains("auth/login.js"))
        .stdout(predicate::str::contains("util/log.js"));

    // Boundary: `sa/` is a real empty dir but must not match `saas/...`. The
    // message must distinguish "empty under this path" from "empty store".
    ynotes()
        .current_dir(dir.path())
        .args(["list", "sa/"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no notes under"));

    // JSON form: the same empty-filter case must surface as a `warnings`
    // entry, so a machine consumer can disambiguate "filter is empty" from
    // "store is empty" without reparsing stderr.
    let out = ynotes()
        .current_dir(dir.path())
        .args(["list", "sa/", "--json"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("valid JSON");
    assert_eq!(v["success"], serde_json::json!(true));
    assert_eq!(
        v["data"]["notes"].as_array().unwrap().len(),
        0,
        "no matches"
    );
    let warnings = v["data"]["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "one warning for the empty filter");
    assert!(
        warnings[0].as_str().unwrap().contains("no notes under"),
        "warning names the empty-under case: {:?}",
        warnings[0]
    );

    // The no-filter case stays clean: an empty filter wasn't passed, so the
    // notes array carries the signal and warnings stays empty.
    let out = ynotes()
        .current_dir(dir.path())
        .args(["list", "--json"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("valid JSON");
    assert_eq!(
        v["data"]["warnings"].as_array().unwrap().len(),
        0,
        "no filter ⇒ no warning needed"
    );
}

/// Saving a path that is outside the store must fail as a **usage** error
/// (exit 2), matching the symlink-escape guard. Before this fix the absolute
/// path form classified as an engine error (exit 1) while the symlink form
/// classified as usage — same user mistake, two different exit codes.
#[test]
fn save_outside_the_store_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    // A sibling tempdir guarantees the path is genuinely outside.
    let outside = tempfile::tempdir().expect("outside tempdir");
    let outside_file = outside.path().join("foo.rs");
    std::fs::write(&outside_file, "fn x() {}\n").unwrap();

    ynotes()
        .current_dir(dir.path())
        .args([
            "save",
            outside_file.to_str().unwrap(),
            "1",
            "-m",
            "should be refused",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("outside the ynotes store"))
        // A non-symlink path must not be blamed on a symlink — the
        // canonical-escape guard at `commands/safety.rs` only fires for
        // genuine symlinks, so this case falls through to the lexical
        // `OutsideStore` and the wording stays accurate.
        .stderr(predicate::str::contains("symlink").not());
}

/// A character device (`/dev/null`) is not a symlink and not a file inside
/// the work tree, but earlier versions rejected it with the symlink-escape
/// guard's "(symlink target escapes the work tree)" wording — false because
/// no symlink was involved. The fix: the symlink guard skips non-symlinks,
/// so `/dev/null` falls through to `Store::relativize` and produces the
/// honest `OutsideStore` message.
///
/// Unix-only: `/dev/null` exists as a character device only here.
#[cfg(unix)]
#[test]
fn save_dev_null_is_outside_the_store_not_a_symlink() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    ynotes()
        .current_dir(dir.path())
        .args(["save", "/dev/null", "-m", "should be refused"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("outside the ynotes store"))
        .stderr(predicate::str::contains("symlink").not());
}

/// `delete` happy path: removing a single note by full id makes it gone.
#[test]
fn delete_removes_a_note_by_full_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\nfn b() {}\n").unwrap();

    let save = ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1", "-m", "first", "--json"])
        .assert()
        .success();
    let save_json: serde_json::Value = serde_json::from_slice(&save.get_output().stdout).unwrap();
    let id = save_json["data"]["id"].as_str().unwrap().to_owned();

    ynotes()
        .current_dir(dir.path())
        .args(["delete", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted"));

    // List sees nothing for that file.
    ynotes()
        .current_dir(dir.path())
        .args(["list", "x.rs"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no notes"));
}

/// A prefix that matches multiple notes is refused, not silently fanned out.
/// We synthesise the collision by forcing the same id-prefix: use a prefix
/// short enough (4 chars) that two unrelated notes are extremely likely to
/// share it. We construct two notes whose ids we know differ.
#[test]
fn delete_refuses_an_ambiguous_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\nfn b() {}\n").unwrap();

    // Two distinct notes with different bodies → different ids.
    let s1 = ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1", "-m", "first", "--json"])
        .assert()
        .success();
    let s2 = ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "2", "-m", "second", "--json"])
        .assert()
        .success();
    let id1: String = serde_json::from_slice::<serde_json::Value>(&s1.get_output().stdout).unwrap()
        ["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let id2: String = serde_json::from_slice::<serde_json::Value>(&s2.get_output().stdout).unwrap()
        ["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Find the longest common prefix of the two ids — that's a prefix that
    // is guaranteed to match both. (Sometimes it is just 0–1 chars; pad to 4
    // by appending characters from id1 until distinct, and back off if that
    // breaks the match.) For determinism, find the LCP and only run the
    // test if it's >= 4; otherwise skip with a note (collision is rare).
    let lcp = id1
        .chars()
        .zip(id2.chars())
        .take_while(|(a, b)| a == b)
        .count();
    if lcp < 4 {
        eprintln!("skip: this run's two ids share only {lcp} leading chars (need >= 4)");
        return;
    }
    let prefix: String = id1.chars().take(lcp).collect();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["delete", &prefix, "--json"])
        .assert()
        .failure()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    // Failure envelope on ambiguous-only? No — ambiguous is a per-id outcome
    // that still emits the success envelope. The exit code is 1; the payload
    // tells the story.
    assert_eq!(v["success"], serde_json::json!(true));
    assert!(!v["data"]["ambiguous"].as_array().unwrap().is_empty());
    assert!(v["data"]["deleted"].as_array().unwrap().is_empty());
}

/// A prefix that matches nothing exits 1 and is listed under `not_found`.
#[test]
fn delete_reports_not_found_and_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let out = ynotes()
        .current_dir(dir.path())
        .args(["delete", "deadbeef", "--json"])
        .assert()
        .failure()
        .code(1);
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["data"]["not_found"][0], "deadbeef");
}

/// A prefix shorter than the 4-char minimum is a usage error — caught up
/// front before touching the store.
#[test]
fn delete_refuses_a_short_prefix_as_usage_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["delete", "abc"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("too short"));
}

/// `--dry-run` reports the would-be deletion but leaves the note in place.
#[test]
fn delete_dry_run_does_not_remove() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\n").unwrap();
    let save = ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1", "-m", "keep me", "--json"])
        .assert()
        .success();
    let id = serde_json::from_slice::<serde_json::Value>(&save.get_output().stdout).unwrap()
        ["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    ynotes()
        .current_dir(dir.path())
        .args(["delete", &id, "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("would delete"));

    // Still there.
    ynotes()
        .current_dir(dir.path())
        .args(["query", "x.rs", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("keep me"));
}

/// `update` rewrites a note's body in place, supersedes the old id, and
/// preserves the note's location.
#[test]
fn update_replaces_body_and_supersedes_old_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let save = ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1", "-m", "old body", "--json"])
        .assert()
        .success();
    let old_id = serde_json::from_slice::<serde_json::Value>(&save.get_output().stdout).unwrap()
        ["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let upd = ynotes()
        .current_dir(dir.path())
        .args(["update", &old_id, "-m", "new body", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&upd.get_output().stdout).unwrap();
    assert_eq!(v["data"]["previous_id"], serde_json::json!(old_id));
    assert_ne!(v["data"]["id"], serde_json::json!(old_id));

    // Querying the location returns the new body, and only it.
    let q = ynotes()
        .current_dir(dir.path())
        .args(["query", "x.rs", "1", "--json"])
        .assert()
        .success();
    let qv: serde_json::Value = serde_json::from_slice(&q.get_output().stdout).unwrap();
    let notes = qv["data"]["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "old note must be retired");
    assert_eq!(notes[0]["body"], "new body");
}

/// `update` with the same body is a no-op: id stays, `created` is false.
#[test]
fn update_with_identical_body_is_a_noop() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\n").unwrap();
    let save = ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1", "-m", "same", "--json"])
        .assert()
        .success();
    let id = serde_json::from_slice::<serde_json::Value>(&save.get_output().stdout).unwrap()
        ["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let upd = ynotes()
        .current_dir(dir.path())
        .args(["update", &id, "-m", "same", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&upd.get_output().stdout).unwrap();
    assert_eq!(v["data"]["id"], v["data"]["previous_id"]);
    assert_eq!(v["data"]["created"], serde_json::json!(false));
}

/// `prune` removes every orphaned note in one pass.
#[test]
fn prune_removes_orphans_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    // Two files, one note each. We'll orphan the second by deleting its file.
    std::fs::write(dir.path().join("keep.rs"), "fn a() {}\n").unwrap();
    std::fs::write(dir.path().join("lose.rs"), "fn b() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "keep.rs", "1", "-m", "keeper"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "lose.rs", "1", "-m", "loser"])
        .assert()
        .success();
    std::fs::remove_file(dir.path().join("lose.rs")).unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["prune", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    let pruned = v["data"]["pruned"].as_array().unwrap();
    assert_eq!(pruned.len(), 1, "exactly one orphan must be pruned");
    assert_eq!(pruned[0]["target"], "lose.rs");
    assert_eq!(v["data"]["kept"], serde_json::json!(1));

    // The keeper survives.
    ynotes()
        .current_dir(dir.path())
        .args(["query", "keep.rs", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("keeper"));
}

/// `prune --dry-run` reports what would go but writes nothing.
#[test]
fn prune_dry_run_leaves_orphans_in_place() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\n").unwrap();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1", "-m", "orphaned soon"])
        .assert()
        .success();
    std::fs::remove_file(dir.path().join("x.rs")).unwrap();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["prune", "--dry-run", "--json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(v["data"]["dry_run"], serde_json::json!(true));
    assert_eq!(v["data"]["pruned"].as_array().unwrap().len(), 1);

    // Still in the store — a fresh prune would still see it.
    let again = ynotes()
        .current_dir(dir.path())
        .args(["prune", "--dry-run", "--json"])
        .assert()
        .success();
    let v2: serde_json::Value = serde_json::from_slice(&again.get_output().stdout).unwrap();
    assert_eq!(v2["data"]["pruned"].as_array().unwrap().len(), 1);
}

/// Parallel saves of identical content must report exactly one `created: true`.
/// Before the noclobber fix, two concurrent callers both saw "file absent"
/// in the TOCTOU window and both wrote, both reporting created=true while
/// the on-disk record was correct. The atomic rename-no-replace closes that
/// window.
///
/// Eight concurrent saves of the same content from the same store directory.
#[test]
fn parallel_identical_saves_report_exactly_one_creation() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\n").unwrap();

    let dir_path = dir.path().to_path_buf();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let d = dir_path.clone();
        handles.push(std::thread::spawn(move || {
            let out = ynotes()
                .current_dir(&d)
                .args(["save", "x.rs", "1", "-m", "racing body", "--json"])
                .assert()
                .success();
            let v: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
            v["data"]["created"].as_bool().unwrap()
        }));
    }
    let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let creations = results.iter().filter(|c| **c).count();
    assert_eq!(
        creations,
        1,
        "noclobber must report exactly one creation across {} racing saves, got {creations}",
        results.len()
    );
}
/// The warning must read as a directory-form ("no notes under … (no such
/// directory)"), not the generic file-form, so the agent isn't told to
/// "check the path" when the path's missingness is the point. Covers the
/// `ends_with_separator` branch of `empty_filter_message` and pins down the
/// trailing-slash-as-intent contract.
#[test]
fn list_trailing_slash_on_a_missing_path_emits_the_directory_form() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    let out = ynotes()
        .current_dir(dir.path())
        .args(["list", "no-such-dir/", "--json"])
        .assert()
        .success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("valid JSON");
    let warnings = v["data"]["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1, "one warning for the missing dir filter");
    let msg = warnings[0].as_str().unwrap();
    assert!(
        msg.contains("no notes under") && msg.contains("no such directory"),
        "trailing slash on a missing path must read as a missing directory: {msg:?}"
    );
}

/// An intra-repo symlink (target resolves back into the work tree) must still
/// work — refusing all symlinks would break legitimate uses like vendored
/// directories or monorepo workspace links.
#[cfg(unix)]
#[test]
fn save_allows_a_symlink_whose_target_stays_inside_the_work_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    let real = dir.path().join("real.txt");
    std::fs::write(&real, "hello\n").unwrap();
    let link = dir.path().join("alias.lnk");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    ynotes()
        .current_dir(dir.path())
        .args(["save", "alias.lnk", "-m", "intra-repo symlink is fine"])
        .assert()
        .success();
}

#[test]
fn list_text_renders_scope_prefix_distinguishing_file_from_range() {
    // A file note over a 3-line file and a range note over its full 1:3 span
    // resolve to the same line range; without a scope marker on the text line,
    // `list` and `query` rendered them identically and the only recoverable
    // signal was `--json`'s `scope` field. The format now matches `save` /
    // `delete` ("file:" / "range:") so a human reader sees the distinction.
    let dir = tempfile::tempdir().expect("tempdir");
    ynotes()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    std::fs::write(dir.path().join("x.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();

    ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "-m", "file-scope body"])
        .assert()
        .success();
    ynotes()
        .current_dir(dir.path())
        .args(["save", "x.rs", "1:3", "-m", "range-scope body"])
        .assert()
        .success();

    ynotes()
        .current_dir(dir.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("x.rs file:1:3"))
        .stdout(predicate::str::contains("x.rs range:1:3"));
}

// --- clap-level errors under `--json` (P1.5a) -------------------------------
//
// The agent contract (ynotes.schema.json) promises the envelope is emitted on
// stdout regardless of exit code. clap rejects bad invocations *before*
// dispatch, so these cases must be routed through the JSON failure envelope
// too — otherwise a `--json` consumer gets empty stdout and a stderr line it
// was told it would never have to read.

/// A missing required argument under `--json` must still yield the failure
/// envelope on stdout (not an empty stdout + a clap diagnostic on stderr).
#[test]
fn clap_missing_arg_with_json_emits_failure_envelope_on_stdout() {
    let out = ynotes()
        .args(["save", "--json"]) // `<FILE>` is required; clap rejects this
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .expect("clap usage errors under --json must emit a valid JSON envelope");
    assert_eq!(parsed["success"], serde_json::json!(false));
    assert_eq!(parsed["v"], serde_json::json!(5));
    assert_eq!(parsed["type"], serde_json::json!("usage"));
    assert!(
        !parsed["error"].as_str().unwrap().is_empty(),
        "error message must be carried in-band: {parsed:?}"
    );
}

/// An unknown subcommand under `--json` is also a clap-level rejection and
/// must ride the same failure envelope on stdout.
#[test]
fn clap_unknown_subcommand_with_json_emits_failure_envelope_on_stdout() {
    let out = ynotes()
        .args(["definitely-not-a-real-command", "--json"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("valid JSON envelope on stdout");
    assert_eq!(parsed["success"], serde_json::json!(false));
    assert_eq!(parsed["v"], serde_json::json!(5));
    assert_eq!(parsed["type"], serde_json::json!("usage"));
}

/// `--help` requested alongside `--json` is NOT an error: help/version are
/// successful outputs and must still print to stdout with exit 0, never wrapped
/// in a failure envelope.
#[test]
fn help_under_json_is_still_plain_help_not_a_failure_envelope() {
    let out = ynotes()
        .args(["save", "--help", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).unwrap();
    assert!(
        stdout.contains("Usage") || stdout.contains("usage"),
        "help text expected on stdout, got: {stdout}"
    );
    assert!(
        !stdout.contains("\"success\""),
        "help must not be rendered as a JSON envelope: {stdout}"
    );
}

/// Without `--json`, a clap usage error keeps its native behaviour: the
/// diagnostic on stderr, stdout empty, exit 2. The fix must not regress the
/// human path.
#[test]
fn clap_usage_error_without_json_keeps_stderr_diagnostic() {
    ynotes()
        .arg("save") // missing `<FILE>`, no --json
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("required"));
}
