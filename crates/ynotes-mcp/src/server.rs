//! The MCP server: tool router + handshake metadata. Handlers call the sync
//! engine through `spawn_blocking` — the engine never sees an async type.

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::forget::ForgetArgs;
use crate::instructions::INSTRUCTIONS;
use crate::notes::NotesArgs;
use crate::reanchor_tool::ReanchorArgs;
use crate::recall::RecallArgs;
use crate::remember::RememberArgs;
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
    /// See spec §5.2 — description text verbatim from the design doc.
    #[tool(
        description = "Save a context note anchored to a code region, committed to the repo and shared with the team. Save the why, not the what: constraints, invariants, gotchas, cross-file couplings, rejected approaches. Only save what would surprise a competent reader in three months - never what the code plainly shows. Anchor to the smallest region the context is about (omit start/end for a whole-file note). Pass code (the region's text exactly as the file reads now) alongside start: stale line numbers are then verified and corrected, so the note cannot land on the wrong region. Saving to the same location supersedes the note there - this is also how you correct one. Creates the note store at the repo root on first use.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remember(&self, Parameters(args): Parameters<RememberArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::remember::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.3 — description text verbatim from the design doc.
    #[tool(
        description = "Delete a note by id. Use when a note is wrong, obsolete, or noise. To correct a note, prefer remember at the same location (it supersedes). dry_run previews. A corrupt/unreadable record is removable only by its exact 64-character id.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn forget(&self, Parameters(args): Parameters<ForgetArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::forget::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.4 — description text verbatim from the design doc.
    #[tool(
        description = "Browse the note store: one note by id, search bodies by substring, or list every note with its current anchor status. For notes on a specific file or region use recall. count: true gives a store-wide status breakdown. Returns {\"store\":\"absent\"} when the repo has no note store.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn notes(&self, Parameters(args): Parameters<NotesArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::notes::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.5 — description text verbatim from the design doc.
    #[tool(
        description = "Persist re-anchors for every note that confidently moved: refreshes selectors, follows committed file renames, collapses merge duplicates. Run after work that moved or heavily edited annotated code, or when recall shows drifted notes. Never touches orphaned notes. dry_run previews. Returns {\"store\":\"absent\"} when the repo has no note store.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn reanchor(&self, Parameters(args): Parameters<ReanchorArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::reanchor_tool::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
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
