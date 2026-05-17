//! # ynotes (the engine)
//!
//! Every behaviour of ynotes lives in this library. The binary
//! (`src/main.rs`) is a thin presentation layer on top of it — it parses
//! arguments and renders what these functions return, nothing more.
//!
//! ## Why a library inside a single crate
//!
//! ynotes ships as one crate today (the binary uses this library directly,
//! the way `bat` is structured). Keeping the engine behind a library boundary
//! *now* — rather than as loose modules in the binary — means that when a
//! second front-end (an MCP server for agents) becomes real, this code lifts
//! into its own crate mechanically, with no behaviour change. The boundary is
//! the asset; the crate split is a later, cheap formality.
//!
//! ## Modules
//!
//! - [`error`] — the single [`Error`] type every fallible operation returns.
//!
//! ## Stability
//!
//! Pre-1.0: the API may change between minor versions. Breaking changes are
//! recorded in `CHANGELOG.md`.

mod error;

pub use error::{Error, Result};

/// Returns the compiled version of ynotes.
///
/// This is the `version` field from `Cargo.toml`, baked in at compile time, so
/// it can never disagree with the artefact it came from. The binary reports it
/// in `ynotes --version` and `ynotes doctor`.
///
/// # Examples
///
/// ```
/// assert!(!ynotes::version().is_empty());
/// ```
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_cargo_metadata() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
    }
}
