//! # ynotes (the engine)
//!
//! Every behaviour of ynotes lives in this library. The binary
//! (`src/main.rs`) is a thin presentation layer on top of it — it parses
//! arguments and renders what these functions return, nothing more.
//!
//! ## Why a library inside a single crate
//!
//! ynotes ships as one crate today (the binary uses this library directly).
//! Keeping the engine behind a library boundary
//! *now* — rather than as loose modules in the binary — means that when a
//! second front-end (an MCP server for agents) becomes real, this code lifts
//! into its own crate mechanically, with no behaviour change. The boundary is
//! the asset; the crate split is a later, cheap formality.
//!
//! ## Modules
//!
//! - `contract` — the versioned `--json` payload shapes every front-end emits.
//! - `error` — the single [`Error`] type every fallible operation returns.
//! - `source` — a file as lines, and the [`LineRange`] primitive.
//! - `path` — the repo-relative, forward-slash path used as a note's identity.
//! - `selector` — the redundant location bundle captured per note.
//! - `note` — a stored note and its content-addressed id.
//! - `store` — the `.ynotes/` content-addressed note store.
//! - `git` — R1's shell-out git transport.
//! - `structural` — R4's tree-sitter structural anchor.
//! - `anchor` — the resolver: selector bundle + file → [`Resolution`].
//! - `query` — which resolved notes a request returns; the orphan promise.
//! - `reanchor` — the explicit, conservative selector-refresh pass.
//!
//! ## Stability
//!
//! Pre-1.0: the API may change between minor versions. Breaking changes are
//! recorded in `CHANGELOG.md`.

// Declaration order mirrors the layering in the module list above, so the
// two cannot drift on a rename.
mod anchor;
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
pub use error::{Error, Result};
pub use git::{GitContext, git_version};
pub use note::{Note, Scope};
pub use query::{
    LineSpec, ListResult, QueryResult, ResolvedNote, list, lookup, query, resolve_scope,
    resolve_scope_verified,
};
pub use reanchor::{
    ReanchorChange, ReanchorRelocation, ReanchorReport, ReanchorSkip, ReanchorSkipReason, reanchor,
};
pub use selector::{GitSelector, SCHEMA, SelectorBundle};
pub use source::{LineRange, SourceFile};
pub use store::{
    IdMatch, IndexHealth, MalformedNote, NoteScan, ReindexReport, Store, StoreHealth,
    validate_id_prefix,
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
