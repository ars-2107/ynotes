//! The MCP server: tool router + handshake metadata. Handlers call the sync
//! engine through `spawn_blocking`, the engine never sees an async type.

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::confirm::ConfirmArgs;
use crate::files::FilesArgs;
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
    /// See spec §5.1, description text verbatim from the design doc.
    #[tool(
        description = "Return the context notes overlapping a file or line region. Requires root: the absolute repository path. Call before editing or reviewing code you did not write this session - code that looks wrong may have a note explaining why it is deliberate. Statuses: anchored (region intact, not proof the claim is true), drifted (region moved or changed - verify against current code), orphaned (region gone - historical only). Notes follow file renames. Returns {\"store\":\"absent\"} when the repo has no note store. Use count: true to cheaply check whether context exists before pulling bodies.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = Arc::clone(&crate::output_schema::RECALL)
    )]
    async fn recall(&self, Parameters(args): Parameters<RecallArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::recall::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.2, description text verbatim from the design doc.
    #[tool(
        description = "Save a context note anchored to a code region, committed to the repo and shared with the team. Requires root: the absolute repository path, where the store is created on first use. Save the why, not the what: constraints, invariants, gotchas, cross-file couplings, rejected approaches. Only save what would surprise a competent reader in three months - never what the code plainly shows. Anchor to the smallest region the context is about (omit start/end for a whole-file note). Pass code (the region's text exactly as the file reads now) alongside start: stale line numbers are then verified and corrected, so the note cannot land on the wrong region. Saving to the same location supersedes the note there - this is also how you correct one. Check neighbours in the result: it lists the other notes already on that file, because superseding only collapses an identical location and a restatement a few lines away would otherwise land silently. If one already says what you just said, delete the new note or fold both into one.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = Arc::clone(&crate::output_schema::REMEMBER)
    )]
    async fn remember(&self, Parameters(args): Parameters<RememberArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::remember::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// The cheap orientation read: where context exists at all.
    #[tool(
        description = "List which files in the repository carry notes, and how many each has. Requires root: the absolute repository path. Call this once when you start work in a repository: recall is otherwise a guess, and knowing which files are annotated is what makes it worth calling on the ones that are. Existing targets need only records and path checks. Missing targets use corroborated Git renames; unresolved notes keep their historical path. Reads never write. Returns no line numbers and no statuses on purpose: those would be stored values, not facts about current code. Use recall for what a note says and where it is now. Files come most-annotated first. Returns {\"store\":\"absent\"} when the repo has no note store.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = Arc::clone(&crate::output_schema::FILES)
    )]
    async fn files(&self, Parameters(args): Parameters<FilesArgs>) -> CallToolResult {
        tokio::task::spawn_blocking(move || crate::files::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// Explicitly acknowledge that a note's prose was checked against code.
    #[tool(
        description = "Mark one note as reviewed against its currently resolved code. Requires root: the absolute repository path. Use only after checking that the note body is still true. Reanchor refreshes selectors but deliberately leaves edited context drifted; confirm is the explicit acknowledgement that restores anchored. A region that cannot be identified is refused: an orphan, or one located only by similarity rather than by git transport, an exact quote, or its structural path. Call reanchor first in that case, then confirm.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = Arc::clone(&crate::output_schema::CONFIRM)
    )]
    async fn confirm(&self, Parameters(args): Parameters<ConfirmArgs>) -> CallToolResult {
        tokio::task::spawn_blocking(move || crate::confirm::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.3, description text verbatim from the design doc.
    #[tool(
        description = "Delete a note by id. Requires root: the absolute repository path. Use when a note is wrong, obsolete, or noise. To correct a note, prefer remember at the same location (it supersedes). dry_run previews. A corrupt/unreadable record is removable only by its exact 64-character id.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        ),
        output_schema = Arc::clone(&crate::output_schema::FORGET)
    )]
    async fn forget(&self, Parameters(args): Parameters<ForgetArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::forget::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.4, description text verbatim from the design doc.
    #[tool(
        description = "Browse the note store: one note by id, search bodies by substring, or list every note with its current anchor status. Requires root: the absolute repository path. For notes on a specific file or region use recall. count: true gives a store-wide status breakdown. Returns {\"store\":\"absent\"} when the repo has no note store.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false),
        output_schema = Arc::clone(&crate::output_schema::NOTES)
    )]
    async fn notes(&self, Parameters(args): Parameters<NotesArgs>) -> CallToolResult {
        // The engine is synchronous; run it on the blocking pool so it never
        // stalls the current-thread runtime driving the stdio transport.
        tokio::task::spawn_blocking(move || crate::notes::run(&args))
            .await
            .unwrap_or_else(|e| tool_err("engine", format!("task join: {e}")))
    }
    /// See spec §5.5, description text verbatim from the design doc.
    #[tool(
        description = "Persist re-anchors for every note that confidently moved: refreshes selectors, follows committed file renames, collapses merge duplicates. Requires root: the absolute repository path. Run after work that moved or heavily edited annotated code, or when recall shows drifted notes. Never touches orphaned notes. review_pending lists notes already anchored correctly whose bodies are still unreviewed - confirm each after checking it. dry_run previews. Returns {\"store\":\"absent\"} when the repo has no note store.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        output_schema = Arc::clone(&crate::output_schema::REANCHOR)
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
    // Override rmcp's generated async method because listing the in-memory
    // router needs no await. A ready future keeps that boundary synchronous.
    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::ListToolsResult, rmcp::ErrorData>>
    + Send
    + '_ {
        std::future::ready(Ok(rmcp::model::ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        }))
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS.to_string())
            // Name the server explicitly: `from_build_env` (rmcp's default)
            // derives `serverInfo.name` from the crate name (`ynotes_mcp`), but
            // clients register and invoke it as `ynotes`.
            .with_server_info(Implementation::new("ynotes", env!("CARGO_PKG_VERSION")))
    }
}
