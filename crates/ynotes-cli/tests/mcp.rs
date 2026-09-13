//! Wire-level MCP tests: spawn the real binary in server mode and speak
//! newline-delimited JSON-RPC over its stdio, the exact surface a client
//! sees. No SDK on the test side, so an rmcp regression cannot mask a wire
//! regression.
//!
//! [`McpSession`] is the shared harness the tool tests (`recall`, `remember`,
//! …) reuse: `tool_call` and the stashed handshake fields (`instructions`,
//! `server_name`, `capabilities`) are the contract those tests speak.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// One spawned `ynotes mcp` process and the JSON-RPC stream to it, plus the
/// handshake facts (`instructions`, `server_name`) stashed at initialize time.
struct McpSession {
    child: Child,
    stdin: ChildStdin,
    /// Lines from the server's stdout, pumped by a detached reader thread so a
    /// hung server times a test out instead of hanging the whole suite. The
    /// thread exits when the child dies and its stdout closes.
    lines: mpsc::Receiver<String>,
    next_id: i64,
    /// The server `instructions` returned by the initialize handshake.
    instructions: String,
    /// `serverInfo.name` from the initialize handshake.
    server_name: String,
    /// The advertised `capabilities` object from the initialize handshake.
    capabilities: serde_json::Value,
    /// Absolute repository root injected into every ordinary tool call.
    root: String,
}

/// Spawn `ynotes mcp` in `workdir`, complete the MCP initialize handshake, and
/// return the live session with its handshake facts stashed.
fn mcp_session(workdir: &Path) -> McpSession {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ynotes"))
        .arg("mcp")
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ynotes mcp");
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in stdout.lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut s = McpSession {
        child,
        stdin,
        lines,
        next_id: 0,
        instructions: String::new(),
        server_name: String::new(),
        capabilities: serde_json::Value::Null,
        root: workdir.to_string_lossy().into_owned(),
    };
    // MCP initialize handshake.
    let init = s.request(
        "initialize",
        &serde_json::json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "ynotes-tests", "version": "0"}
        }),
    );
    assert!(init.get("serverInfo").is_some(), "no serverInfo in {init}");
    init["instructions"]
        .as_str()
        .unwrap_or_default()
        .clone_into(&mut s.instructions);
    init["serverInfo"]["name"]
        .as_str()
        .unwrap_or_default()
        .clone_into(&mut s.server_name);
    s.capabilities = init["capabilities"].clone();
    s.notify("notifications/initialized");
    s
}

impl McpSession {
    /// Send a JSON-RPC request and return its `result`, skipping any
    /// server-initiated notifications that arrive first.
    fn request(&mut self, method: &str, params: &serde_json::Value) -> serde_json::Value {
        self.next_id += 1;
        let frame = serde_json::json!(
            {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params});
        writeln!(self.stdin, "{frame}").unwrap();
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .expect("server response within 30s");
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            // Skip server-initiated notifications; return our response.
            if v.get("id") == Some(&serde_json::json!(self.next_id)) {
                assert!(v.get("error").is_none(), "rpc error: {v}");
                return v["result"].clone();
            }
        }
    }

    /// Send a JSON-RPC notification (no id, no response expected).
    fn notify(&mut self, method: &str) {
        let frame = serde_json::json!({"jsonrpc": "2.0", "method": method});
        writeln!(self.stdin, "{frame}").unwrap();
    }

    /// Invoke an MCP tool by name and return the `tools/call` result.
    fn tool_call(&mut self, name: &str, args: &serde_json::Value) -> serde_json::Value {
        let mut arguments = args.clone();
        arguments
            .as_object_mut()
            .expect("tool arguments are an object")
            .insert("root".to_owned(), serde_json::json!(self.root));
        self.request(
            "tools/call",
            &serde_json::json!({"name": name, "arguments": arguments}),
        )
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        // Best-effort teardown: the process is going away either way, so a
        // failed kill/wait has nothing to report to. `drop`, not `let _`, to
        // satisfy the workspace `let_underscore_drop` lint.
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

/// Run the `ynotes` binary in `dir`, asserting it exits successfully. Used to
/// build a real store (`init`) and populate it (`save`) before the server sees
/// it, the wire tests exercise the same on-disk artefacts a client would.
fn run_cli(dir: &Path, args: &[&str]) {
    let status = Command::new(env!("CARGO_BIN_EXE_ynotes"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run ynotes");
    assert!(status.success(), "`ynotes {args:?}` failed");
}

/// A tempdir holding a git-less store with one range note (lines 2-3 of
/// `f.txt`) whose body is `body`, built through the real CLI.
fn repo_with_note(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), "alpha\nbravo\ncharlie\ndelta\n").unwrap();
    run_cli(dir.path(), &["init"]);
    run_cli(dir.path(), &["save", "f.txt", "2:3", "-m", body]);
    dir
}

#[test]
fn recall_returns_saved_note_body() {
    let dir = repo_with_note("deliberate: leave the retry loop");
    let mut s = mcp_session(dir.path());
    let result = s.tool_call("recall", &serde_json::json!({"file": "f.txt"}));
    assert_ne!(
        result["isError"],
        serde_json::json!(true),
        "tool error: {result}"
    );
    let text = result["content"][0]["text"].as_str().expect("text block");
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        payload["notes"][0]["body"],
        "deliberate: leave the retry loop"
    );
}

#[test]
fn recall_count_mode_reports_total() {
    let dir = repo_with_note("x");
    let mut s = mcp_session(dir.path());
    let result = s.tool_call(
        "recall",
        &serde_json::json!({"file": "f.txt", "count": true}),
    );
    let text = result["content"][0]["text"].as_str().expect("text block");
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["count"]["total"], 1);
}

#[test]
fn recall_without_store_is_store_absent_success() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.txt"), "alpha\n").unwrap();
    let mut s = mcp_session(dir.path());
    let result = s.tool_call("recall", &serde_json::json!({"file": "f.txt"}));
    assert_ne!(
        result["isError"],
        serde_json::json!(true),
        "store-absent must be a success: {result}"
    );
    let text = result["content"][0]["text"].as_str().expect("text block");
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["store"], "absent");
}

#[test]
fn remember_then_recall_round_trips_in_one_session() {
    // A repo with a git root but no store yet: `remember` must bootstrap one.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join("f.txt"), "alpha\nbravo\ncharlie\ndelta\n").unwrap();
    let mut s = mcp_session(dir.path());

    let saved = s.tool_call(
        "remember",
        &serde_json::json!({
            "file": "f.txt",
            "start": 2,
            "end": 3,
            "body": "deliberate: keep the retry loop"
        }),
    );
    assert_ne!(
        saved["isError"],
        serde_json::json!(true),
        "remember errored: {saved}"
    );
    let saved_text = saved["content"][0]["text"].as_str().expect("text block");
    let saved_payload: serde_json::Value = serde_json::from_str(saved_text).unwrap();
    assert_eq!(
        saved_payload["store_created"], true,
        "first save must bootstrap the store"
    );
    assert!(
        dir.path().join(".ynotes").is_dir(),
        ".ynotes store not created on disk"
    );

    // Same session: recall must see the note just remembered, the loop closed.
    let recalled = s.tool_call("recall", &serde_json::json!({"file": "f.txt"}));
    assert_ne!(
        recalled["isError"],
        serde_json::json!(true),
        "recall errored: {recalled}"
    );
    let recalled_text = recalled["content"][0]["text"].as_str().expect("text block");
    let recalled_payload: serde_json::Value = serde_json::from_str(recalled_text).unwrap();
    assert_eq!(
        recalled_payload["notes"][0]["body"],
        "deliberate: keep the retry loop"
    );
}

#[test]
fn forget_then_recall_shows_the_note_gone() {
    // Drive the whole loop over the wire: remember returns the id, forget
    // removes it, and a follow-up recall must find nothing.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join("f.txt"), "alpha\nbravo\ncharlie\ndelta\n").unwrap();
    let mut s = mcp_session(dir.path());

    let saved = s.tool_call(
        "remember",
        &serde_json::json!({"file": "f.txt", "start": 2, "end": 3, "body": "temporary note"}),
    );
    let saved_text = saved["content"][0]["text"].as_str().expect("text block");
    let saved_payload: serde_json::Value = serde_json::from_str(saved_text).unwrap();
    let id = saved_payload["id"].as_str().expect("saved id").to_owned();

    let forgotten = s.tool_call("forget", &serde_json::json!({"id": id}));
    assert_ne!(
        forgotten["isError"],
        serde_json::json!(true),
        "forget errored: {forgotten}"
    );
    let forgotten_text = forgotten["content"][0]["text"]
        .as_str()
        .expect("text block");
    let forgotten_payload: serde_json::Value = serde_json::from_str(forgotten_text).unwrap();
    assert_eq!(forgotten_payload["deleted"][0]["id"], id);

    let recalled = s.tool_call("recall", &serde_json::json!({"file": "f.txt"}));
    let recalled_text = recalled["content"][0]["text"].as_str().expect("text block");
    let recalled_payload: serde_json::Value = serde_json::from_str(recalled_text).unwrap();
    assert_eq!(
        recalled_payload["notes"]
            .as_array()
            .expect("notes array")
            .len(),
        0,
        "the note should be gone after forget: {recalled_payload}"
    );
}

#[test]
fn notes_lists_what_two_remembers_created() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join("f.txt"), "alpha\nbravo\ncharlie\ndelta\n").unwrap();
    let mut s = mcp_session(dir.path());

    s.tool_call(
        "remember",
        &serde_json::json!({"file": "f.txt", "start": 1, "end": 1, "body": "note one"}),
    );
    s.tool_call(
        "remember",
        &serde_json::json!({"file": "f.txt", "start": 3, "end": 4, "body": "note two"}),
    );

    let listed = s.tool_call("notes", &serde_json::json!({}));
    assert_ne!(
        listed["isError"],
        serde_json::json!(true),
        "notes errored: {listed}"
    );
    let text = listed["content"][0]["text"].as_str().expect("text block");
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    let notes = payload["notes"].as_array().expect("notes array");
    assert_eq!(notes.len(), 2, "expected both remembered notes: {payload}");
    let bodies: Vec<&str> = notes.iter().map(|n| n["body"].as_str().unwrap()).collect();
    assert!(
        bodies.contains(&"note one") && bodies.contains(&"note two"),
        "both bodies should be listed: {bodies:?}"
    );
}

/// The reviewed contract for the whole tool surface: exactly seven tools,
/// their verbatim descriptions, their annotations, and each tool's
/// `outputSchema` (assembled from the published schema's `$defs`, see
/// `ynotes-mcp/src/output_schema.rs`). Freezing it here means any future
/// change to a tool's name, description, annotations, or declared output
/// shape is reviewed as a snapshot diff (invariant #7 for the MCP face)
/// rather than shipped silently.
#[test]
fn tools_list_is_the_reviewed_contract() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = mcp_session(dir.path());
    let mut tools = s.request("tools/list", &serde_json::json!({}));
    // Sort by name for determinism before snapshotting.
    tools["tools"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(|t| t["name"].as_str().unwrap().to_owned());
    insta::assert_json_snapshot!("mcp_tools_list", tools);
}

#[test]
fn every_tool_requires_an_explicit_repository_root() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = mcp_session(dir.path());
    let tools = session.request("tools/list", &serde_json::json!({}));
    for tool in tools["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(
            required.iter().any(|field| field == "root"),
            "{name} does not require root: {}",
            tool["inputSchema"]
        );
        assert_eq!(tool["inputSchema"]["properties"]["root"]["type"], "string");
    }
}

/// Validate a tool's text payload against a named `$def` in the published
/// schema. The MCP tools answer with the *bare* contract payload (no envelope),
/// so each is checked against its `$defs` sub-schema directly, the second-face
/// counterpart to the CLI's `schema_conformance` suite, closing invariant #7
/// (the contract cannot fork silently across the two front-ends).
fn assert_conforms(def: &str, payload: &serde_json::Value) {
    let root: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ynotes.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let sub = serde_json::json!({"$ref": format!("#/$defs/{def}"), "$defs": root["$defs"]});
    let compiled = jsonschema::validator_for(&sub).unwrap();
    let errors: Vec<String> = compiled
        .iter_errors(payload)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{def} violations: {errors:?}");
}

/// Validate a result's `structuredContent` against the tool's *declared*
/// `outputSchema` from `tools/list`, the MCP-spec obligation, and the weld
/// that keeps what the surface declares equal to what it emits.
fn assert_matches_declared_schema(
    tools: &serde_json::Value,
    tool: &str,
    result: &serde_json::Value,
) {
    let schema = tools["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|t| t["name"] == tool)
        .unwrap_or_else(|| panic!("no tool named {tool}"))["outputSchema"]
        .clone();
    let compiled = jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("{tool} outputSchema is not a valid schema: {e}"));
    let errors: Vec<String> = compiled
        .iter_errors(&result["structuredContent"])
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "{tool} structuredContent violates its declared outputSchema: {errors:?}"
    );
}

/// Parse a `tools/call` result's single text content block as JSON, every
/// ynotes tool answers with exactly one block carrying the compact payload,
/// and assert `structuredContent` mirrors the same value, so the two faces
/// of one result can never diverge.
fn payload_of(result: &serde_json::Value) -> serde_json::Value {
    let text = result["content"][0]["text"].as_str().expect("text block");
    let payload: serde_json::Value =
        serde_json::from_str(text).expect("tool payload is valid JSON");
    assert_eq!(
        result["structuredContent"], payload,
        "structuredContent must mirror the text block"
    );
    payload
}

/// Every tool's real over-the-wire payload conforms to the published schema.
/// Drives one live session (remember → confirm → recall → recall count → notes
/// → notes count → forget dry-run → reanchor dry-run), then a bare tempdir for the
/// store-absent shape a read answers in a non-adopting repo.
#[test]
fn every_tool_payload_conforms_to_the_published_schema() {
    // A git root but no store yet: `remember` bootstraps one, then every read
    // and the maintenance pass are validated against their schema `$def` AND
    // against the outputSchema the tool itself declares.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join("f.txt"), "alpha\nbravo\ncharlie\ndelta\n").unwrap();
    let mut s = mcp_session(dir.path());
    let tools = s.request("tools/list", &serde_json::json!({}));

    let saved = s.tool_call(
        "remember",
        &serde_json::json!({"file": "f.txt", "start": 2, "end": 3, "body": "load-bearing"}),
    );
    assert_conforms("saveData", &payload_of(&saved));
    assert_matches_declared_schema(&tools, "remember", &saved);
    let id = payload_of(&saved)["id"]
        .as_str()
        .expect("saved id")
        .to_owned();

    let confirmed = s.tool_call("confirm", &serde_json::json!({"id": id}));
    assert_conforms("confirmData", &payload_of(&confirmed));
    assert_matches_declared_schema(&tools, "confirm", &confirmed);

    let recalled = s.tool_call("recall", &serde_json::json!({"file": "f.txt"}));
    assert_conforms("queryData", &payload_of(&recalled));
    assert_matches_declared_schema(&tools, "recall", &recalled);

    let counted = s.tool_call(
        "recall",
        &serde_json::json!({"file": "f.txt", "count": true}),
    );
    assert_conforms("queryCountData", &payload_of(&counted));
    assert_matches_declared_schema(&tools, "recall", &counted);

    let listed = s.tool_call("notes", &serde_json::json!({}));
    assert_conforms("listData", &payload_of(&listed));
    assert_matches_declared_schema(&tools, "notes", &listed);

    let note_count = s.tool_call("notes", &serde_json::json!({"count": true}));
    assert_conforms("listCountData", &payload_of(&note_count));
    assert_matches_declared_schema(&tools, "notes", &note_count);

    let forgotten = s.tool_call("forget", &serde_json::json!({"id": id, "dry_run": true}));
    assert_conforms("deleteData", &payload_of(&forgotten));
    assert_matches_declared_schema(&tools, "forget", &forgotten);

    let reanchored = s.tool_call("reanchor", &serde_json::json!({"dry_run": true}));
    assert_conforms("reanchorData", &payload_of(&reanchored));
    assert_matches_declared_schema(&tools, "reanchor", &reanchored);

    // A pure read in a bare tempdir (no store) answers the storeAbsent shape,
    // which the declared schema must also admit.
    let bare = tempfile::tempdir().unwrap();
    let mut b = mcp_session(bare.path());
    let b_tools = b.request("tools/list", &serde_json::json!({}));
    let absent = b.tool_call("notes", &serde_json::json!({}));
    assert_conforms("storeAbsent", &payload_of(&absent));
    assert_matches_declared_schema(&b_tools, "notes", &absent);
}

/// Error results are prose for the caller, not contract payloads: they must
/// not carry `structuredContent` (the MCP outputSchema obligation applies
/// only to structured results).
#[test]
fn tool_errors_carry_no_structured_content() {
    let dir = repo_with_note("body");
    let mut s = mcp_session(dir.path());
    let result = s.tool_call("forget", &serde_json::json!({"id": "not-hex!"}));
    assert_eq!(
        result["isError"],
        serde_json::json!(true),
        "expected an in-band tool error: {result}"
    );
    assert!(
        result.get("structuredContent").is_none(),
        "errors are text-only: {result}"
    );
}

#[test]
fn handshake_reports_instructions_and_tools_capability() {
    let dir = tempfile::tempdir().unwrap();
    let s = mcp_session(dir.path());
    assert!(
        s.instructions.contains("PROTOCOL"),
        "instructions missing PROTOCOL: {}",
        s.instructions
    );
    assert_eq!(s.server_name, "ynotes", "unexpected serverInfo.name");
    assert!(
        s.capabilities.get("tools").is_some(),
        "tools capability not advertised: {}",
        s.capabilities
    );
}
