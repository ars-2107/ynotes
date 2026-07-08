//! The MCP server: tool router + handshake metadata. Handlers call the sync
//! engine through `spawn_blocking` — the engine never sees an async type.

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::instructions::INSTRUCTIONS;
use crate::recall::RecallArgs;
use crate::tools::tool_err;

/// One instance serves one stdio client (the spawning agent session).
#[derive(Clone)]
pub(crate) struct YnotesServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl YnotesServer {
    /// Build a server with its tool router.
    pub(crate) fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
    /// See spec §5.1 — description text verbatim from the design doc.
    #[tool(
        description = "Return the context notes overlapping a file or line region. Call before editing or reviewing code you did not write this session - code that looks wrong may have a note explaining why it is deliberate. Statuses: anchored (trust it), drifted (region moved or changed - verify against current code), orphaned (region gone - historical only). Notes follow file renames. Returns {\"store\":\"absent\"} when the repo has no note store. Use count: true to cheaply check whether context exists before pulling bodies.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn recall(&self, Parameters(args): Parameters<RecallArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::recall::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    // Remaining tools land in later tasks: remember, forget, notes, reanchor.
}

// `router = self.tool_router` binds the generated `call_tool`/`list_tools`
// handlers to the stored router; the bare form rebuilds via `Self::tool_router()`
// each call and would leave the field unread.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for YnotesServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS.to_string())
            // Name the server explicitly: `from_build_env` (rmcp's default)
            // derives `serverInfo.name` from the crate name (`ynotes_mcp`), but
            // clients register and invoke it as `ynotes`.
            .with_server_info(Implementation::new("ynotes", env!("CARGO_PKG_VERSION")))
    }
}
