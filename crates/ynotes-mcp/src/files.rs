//! Read-only annotated-path inventory, including corroborated pending renames.

use rmcp::model::CallToolResult;
use serde::Deserialize;
use ynotes::contract::FilesData;

use crate::tools::{discover_for, explicit_root, ok_json, store_absent, tool_err};

/// Arguments for the `files` tool.
#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(crate) struct FilesArgs {
    /// Absolute path to the repository root this request addresses.
    pub(crate) root: String,
}

/// List the annotated files for `args.root`.
pub(crate) fn run(args: &FilesArgs) -> CallToolResult {
    let root = match explicit_root(&args.root) {
        Ok(root) => root,
        Err(message) => return tool_err("usage", message),
    };
    let store = match discover_for(&root) {
        Ok(Some(store)) => store,
        Ok(None) => return ok_json(&store_absent()),
        Err(e) => return tool_err("engine", e),
    };
    match ynotes::files(&store) {
        Ok(inventory) => ok_json(&FilesData::new(&inventory)),
        Err(e) => tool_err("engine", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unadopted repository answers the store-absent success, not an error:
    /// an ambient server must not error-spam repositories that never adopted
    /// ynotes, and "there is no context here" is a useful answer.
    #[test]
    fn a_repository_without_a_store_is_a_store_absent_success() {
        let dir = tempfile::tempdir().unwrap();
        let args = FilesArgs {
            root: dir.path().display().to_string(),
        };
        let result = run(&args);
        assert_ne!(result.is_error, Some(true), "absence is not a failure");
    }

    /// A relative root is refused up front (invariant #13): the server must
    /// never resolve a repository from its own process cwd.
    #[test]
    fn a_relative_root_is_a_usage_error() {
        let args = FilesArgs {
            root: "some/relative/path".to_owned(),
        };
        assert_eq!(run(&args).is_error, Some(true));
    }
}
