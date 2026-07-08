//! Wire-level MCP tests: spawn the real binary in server mode and speak
//! newline-delimited JSON-RPC over its stdio — the exact surface a client
//! sees. No SDK on the test side, so an rmcp regression cannot mask a wire
//! regression.
//!
//! [`McpSession`] is the shared harness the tool tests (`recall`, `remember`,
//! …) reuse: `tool_call` and the stashed handshake fields (`instructions`,
//! `server_name`, `capabilities`) are the contract those tests speak.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

/// One spawned `ynotes mcp` process and the JSON-RPC stream to it, plus the
/// handshake facts (`instructions`, `server_name`) stashed at initialize time.
struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
    /// The server `instructions` returned by the initialize handshake.
    instructions: String,
    /// `serverInfo.name` from the initialize handshake.
    server_name: String,
    /// The advertised `capabilities` object from the initialize handshake.
    capabilities: serde_json::Value,
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
    let mut s = McpSession {
        child,
        stdin,
        stdout,
        next_id: 0,
        instructions: String::new(),
        server_name: String::new(),
        capabilities: serde_json::Value::Null,
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
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
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
        self.request(
            "tools/call",
            &serde_json::json!({"name": name, "arguments": args}),
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
/// it — the wire tests exercise the same on-disk artefacts a client would.
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
