//! # ynotes-mcp
//!
//! The MCP (Model Context Protocol) front-end: a stdio JSON-RPC server
//! exposing the ynotes engine to agent clients. This crate is where async
//! enters the workspace — the engine (`ynotes-core`) stays synchronous and
//! is called via `spawn_blocking`. stdout carries protocol frames only;
//! diagnostics go to stderr.

mod forget;
mod instructions;
mod notes;
mod recall;
mod remember;
mod server;
mod tools;

/// Failures starting or running the server (transport-level; per-tool
/// failures are in-band MCP tool errors, never process exits).
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The tokio runtime could not be built.
    #[error("failed to start async runtime: {0}")]
    Runtime(std::io::Error),
    /// The stdio transport failed while serving.
    #[error("mcp transport error: {0}")]
    Transport(String),
}

/// Serve MCP over stdio until the client disconnects. Blocks the calling
/// thread; the binary's `mcp` subcommand is the only intended caller.
///
/// # Errors
///
/// [`ServeError`] on runtime construction or transport failure.
pub fn serve() -> Result<(), ServeError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(ServeError::Runtime)?;
    rt.block_on(async {
        use rmcp::ServiceExt as _;
        let service = server::YnotesServer::new()
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| ServeError::Transport(e.to_string()))?;
        service
            .waiting()
            .await
            .map_err(|e| ServeError::Transport(e.to_string()))?;
        Ok(())
    })
}
