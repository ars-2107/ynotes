//! The MCP server: tool router + handshake metadata. Handlers call the sync
//! engine through `spawn_blocking` — the engine never sees an async type.

use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool_handler, tool_router,
};

use crate::instructions::INSTRUCTIONS;

/// One instance serves one stdio client (the spawning agent session).
#[derive(Clone)]
pub(crate) struct YnotesServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl YnotesServer {
    /// Build a server with its (currently empty) tool router.
    pub(crate) fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
    // Tools land here in later tasks: recall, remember, forget, notes, reanchor.
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
