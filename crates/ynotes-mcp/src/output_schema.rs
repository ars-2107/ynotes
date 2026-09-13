//! Per-tool `outputSchema` assembled from the published contract schema.
//!
//! `ynotes.schema.json` is the single source of truth for every payload
//! (invariant #7). Each tool's `outputSchema` is assembled from it on first
//! use: the union of the `$defs` that tool can answer with, carrying the
//! transitive closure of every `$def` they reference so the document is
//! self-contained. Hand-writing these shapes here would let the declared
//! contract fork from the published one.

use std::sync::{Arc, LazyLock};

use rmcp::model::JsonObject;

/// The published contract schema, embedded at compile time so the installed
/// binary needs no runtime file. `include_str!` keeps the repo file the only
/// authority, editing it re-renders every `outputSchema`.
static SCHEMA: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../ynotes.schema.json"))
        .expect("ynotes.schema.json is valid JSON")
});

/// `recall`: a query payload, its count form, or the store-absent success.
pub(crate) static RECALL: LazyLock<Arc<JsonObject>> =
    LazyLock::new(|| for_defs(&["queryData", "queryCountData", "storeAbsent"]));
/// `remember`: always a save payload (it bootstraps a missing store).
pub(crate) static REMEMBER: LazyLock<Arc<JsonObject>> = LazyLock::new(|| for_defs(&["saveData"]));
/// `confirm`: always an explicit confirmation payload.
pub(crate) static CONFIRM: LazyLock<Arc<JsonObject>> = LazyLock::new(|| for_defs(&["confirmData"]));
/// `forget`: always a delete payload (store-absent is an error for a write).
pub(crate) static FORGET: LazyLock<Arc<JsonObject>> = LazyLock::new(|| for_defs(&["deleteData"]));
/// `notes`: one note, a lookup, the inventory or its count form, or the
/// store-absent success.
pub(crate) static NOTES: LazyLock<Arc<JsonObject>> = LazyLock::new(|| {
    for_defs(&[
        "showData",
        "lookupData",
        "listData",
        "listCountData",
        "storeAbsent",
    ])
});
/// `files`: the annotated-file inventory or the store-absent success.
pub(crate) static FILES: LazyLock<Arc<JsonObject>> =
    LazyLock::new(|| for_defs(&["filesData", "storeAbsent"]));
/// `reanchor`: a re-anchor report or the store-absent success.
pub(crate) static REANCHOR: LazyLock<Arc<JsonObject>> =
    LazyLock::new(|| for_defs(&["reanchorData", "storeAbsent"]));

/// Assemble a self-contained JSON Schema document whose value space is the
/// union of the named `$defs`: a bare `$ref` for one, `anyOf` for several,
/// plus the transitive closure of every `$def` they reach.
///
/// # Panics
///
/// If `defs` is empty, a document with no members would emit `"anyOf": []`,
/// which is invalid under JSON Schema 2020-12; every call site names at least
/// one `$def`. Also panics if a name is missing from the published schema, a
/// rename there must be mirrored here. The statics are all forced by tests, so
/// the panic fires in CI, never first in a client session.
fn for_defs(defs: &[&str]) -> Arc<JsonObject> {
    assert!(
        !defs.is_empty(),
        "for_defs needs at least one $def: an empty union emits invalid `anyOf: []`"
    );
    let all = SCHEMA["$defs"].as_object().expect("schema has $defs");
    let mut doc = JsonObject::new();
    // The MCP spec requires `outputSchema` to declare `type: "object"` at its
    // root (`Tool.properties.outputSchema` in schema/2025-06-18/schema.json:
    // `required: ["type"]`, `type` a `const "object"`). Conjoining it with the
    // `$ref`/`anyOf` below is semantically neutral, under JSON Schema 2020-12
    // keywords beside `$ref` apply in conjunction, and every union member is
    // itself an object shape, so this narrows nothing. `$schema` pins the
    // dialect, matching the published `ynotes.schema.json` and the dialect the
    // rmcp-generated `inputSchema` blocks already carry.
    doc.insert(
        "$schema".into(),
        "https://json-schema.org/draft/2020-12/schema".into(),
    );
    doc.insert("type".into(), "object".into());
    if let [only] = defs {
        doc.insert("$ref".into(), format!("#/$defs/{only}").into());
    } else {
        let refs: Vec<serde_json::Value> = defs
            .iter()
            .map(|d| serde_json::json!({"$ref": format!("#/$defs/{d}")}))
            .collect();
        doc.insert("anyOf".into(), refs.into());
    }
    // Depth-first closure over `$ref`s so nested references (a note's rungs,
    // targets, ranges, …) resolve inside this document alone.
    let mut included = JsonObject::new();
    let mut queue: Vec<String> = defs.iter().map(ToString::to_string).collect();
    while let Some(name) = queue.pop() {
        if included.contains_key(&name) {
            continue;
        }
        let def = all
            .get(&name)
            .unwrap_or_else(|| panic!("unknown $def `{name}` in ynotes.schema.json"))
            .clone();
        collect_refs(&def, &mut queue);
        included.insert(name, def);
    }
    doc.insert("$defs".into(), included.into());
    Arc::new(doc)
}

/// Push every `#/$defs/<name>` referenced anywhere inside `value` onto `queue`.
fn collect_refs(value: &serde_json::Value, queue: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                if key == "$ref"
                    && let Some(name) = inner.as_str().and_then(|r| r.strip_prefix("#/$defs/"))
                {
                    queue.push(name.to_owned());
                }
                collect_refs(inner, queue);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_refs(item, queue);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ALL: [&LazyLock<Arc<JsonObject>>; 7] = [
        &FILES, &RECALL, &REMEMBER, &CONFIRM, &FORGET, &NOTES, &REANCHOR,
    ];

    /// Force every static, a `$def` renamed in the published schema panics
    /// here in CI, never first in a client session.
    #[test]
    fn every_tool_schema_assembles() {
        for schema in ALL {
            assert!(!schema.is_empty());
            // The MCP spec requires `type: "object"` at every `outputSchema`
            // root (see `for_defs`); a client validating `tools/list` rejects
            // a tool without it.
            assert_eq!(schema.get("type").and_then(|t| t.as_str()), Some("object"));
        }
    }

    /// Every `$ref` inside an assembled document resolves within that
    /// document, the self-containedness `outputSchema` demands (a client
    /// cannot see the published file).
    #[test]
    fn assembled_schemas_are_self_contained() {
        for schema in ALL {
            let doc = serde_json::Value::Object((***schema).clone());
            let defs = doc["$defs"].as_object().expect("$defs present");
            let mut refs = Vec::new();
            collect_refs(&doc, &mut refs);
            for name in refs {
                assert!(defs.contains_key(&name), "unresolved $ref `{name}`");
            }
        }
    }

    /// The closure prunes what a tool cannot answer with: another tool's
    /// payload, and the CLI-only envelope shapes.
    #[test]
    fn unrelated_defs_are_pruned() {
        let defs = serde_json::Value::Object((**REMEMBER).clone());
        let defs = defs["$defs"].as_object().unwrap().clone();
        assert!(defs.contains_key("saveData"));
        assert!(
            !defs.contains_key("queryData"),
            "remember's outputSchema must not carry other tools' payloads"
        );
        assert!(
            !defs.contains_key("successEnvelope"),
            "the CLI envelope is not part of any MCP payload"
        );
    }
}
