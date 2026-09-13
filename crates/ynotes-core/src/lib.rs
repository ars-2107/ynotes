//! # ynotes
//!
//! Synchronous storage, anchoring, and query APIs shared by the CLI and MCP server.
//! [`Store`] manages note records; [`SelectorBundle`] captures a code region;
//! [`Resolution`] reports its current location and status. Reads never write.
//!
//! Pre-1.0 APIs may change between minor versions. See `CHANGELOG.md`.

mod anchor;
mod confirm;
pub mod contract;
mod error;
mod git;
mod note;
mod path;
mod query;
mod reanchor;
mod selector;
mod source;
mod store;
mod structural;

pub use anchor::{AnchorStatus, Resolution, Rung, RungOutcome, RungResult, resolve};
pub use confirm::{ConfirmResult, confirm};
pub use error::{Error, Result};
pub use git::{GitContext, git_version};
pub use note::{Note, ReviewBasis, Scope};
pub use query::{
    AnnotatedFile, FileInventory, LineSpec, ListResult, QueryResult, ResolvedNote, files, list,
    lookup, query, resolve_note, resolve_one, resolve_scope, resolve_scope_verified,
};
pub use reanchor::{
    ReanchorChange, ReanchorPending, ReanchorRelocation, ReanchorReport, ReanchorSkip,
    ReanchorSkipReason, reanchor,
};
pub use selector::{GitSelector, SCHEMA, SelectorBundle};
pub use source::{LineRange, SourceFile};
pub use store::{
    IdMatch, IndexHealth, IndexStatus, MalformedNote, NoteScan, ReindexReport, SaveOutcome, Store,
    StoreHealth, ambiguous_id_message, validate_id_prefix,
};
pub use structural::{Language, PathStep, StructuralSelector, capture as structural_capture};

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
