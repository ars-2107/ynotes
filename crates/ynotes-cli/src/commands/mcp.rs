//! `ynotes mcp` — run the stdio MCP server for agent clients.
//!
//! A thin bridge: it hands control to [`ynotes_mcp::serve`], which owns the
//! async runtime and the stdio transport and blocks until the client
//! disconnects. All behaviour lives in the `ynotes-mcp` front-end crate; this
//! module only classifies a start-up/transport failure into an exit code.

use crate::command_error::CommandError;

/// Serve MCP over stdio until the spawning client disconnects.
///
/// # Errors
///
/// Returns [`CommandError::Mcp`] if the async runtime cannot be built or the
/// stdio transport fails while serving (exit `1`).
pub(crate) fn run() -> Result<(), CommandError> {
    ynotes_mcp::serve()?;
    Ok(())
}
