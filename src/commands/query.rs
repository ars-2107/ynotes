//! `ynotes query` — resolve and print the context a request overlaps.
//!
//! Rendering lives in [`super::render`]; this module parses the location,
//! calls the engine, and chooses text vs JSON. Both forms always include the
//! orphan section — a query must never hide that context was lost.

use std::path::Path;
use std::str::FromStr as _;

use serde::Serialize;
use ynotes::{LineSpec, QueryResult, Store};

use super::json_envelope;
use super::render::{
    MalformedView, NoteView, malformed_view, note_view, write_text_malformed, write_text_note,
};
use crate::command_error::CommandError;

/// Resolve notes for `file` at `at` and print them.
///
/// # Errors
///
/// [`CommandError::Usage`] for a bad location; [`CommandError::Engine`] /
/// [`CommandError::Io`] otherwise. Under `--json`, any failure is rendered as
/// the agent-contract failure envelope on stdout and returned as
/// [`CommandError::Rendered`] so main does not also print to stderr.
pub(crate) fn run(
    file: &Path,
    at: Option<&str>,
    json: bool,
    explain: bool,
) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_json(file, at, explain))
    } else {
        run_text(file, at, explain)
    }
}

#[derive(Serialize)]
struct QueryData<'a> {
    query: QuerySpecView<'a>,
    /// Every resolved note this query returns, in a single array — anchored,
    /// drifted, and orphaned together. Each entry carries `status` so a
    /// consumer that wants the historical matched/orphaned split groups by
    /// it (`status == "orphaned"` ⇒ was an orphan; everything else matched
    /// the queried interval under its scope). Unified in `v=5` so query and
    /// `list` share one shape, with orphans still always returned (the
    /// never-drop promise — invariant #4).
    notes: Vec<NoteView>,
    /// Records for this file the index pointed at that could not be read or
    /// parsed. Skipped from `notes` (they cannot be resolved) but surfaced here
    /// so a corrupt record is never silently dropped from a query (invariant
    /// #4). Always present; empty when there is nothing to flag.
    malformed: Vec<MalformedView>,
    /// Non-fatal advisories — populated, for example, when the target file
    /// does not exist (almost always a typo, but `exit=0` is the right Unix
    /// signal for "no match" so the warning lives in-band). Always present,
    /// empty when there is nothing to flag, so consumers do not have to
    /// branch on key presence.
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct QuerySpecView<'a> {
    file: String,
    at: &'a str,
}

fn run_json(file: &Path, at: Option<&str>, explain: bool) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let spec =
        LineSpec::from_str(at.unwrap_or("")).map_err(|e| CommandError::Usage(e.to_string()))?;
    // `?` so an `OutsideStore` from the engine surfaces as a usage error
    // (exit `2`) rather than an engine error (exit `1`).
    let result = ynotes::query(&store, file, spec)?;

    let mut warnings: Vec<String> = Vec::new();
    if !file.exists() {
        warnings.push(format!(
            "`{}` does not exist (check the path)",
            file.display()
        ));
    }
    // Matched first (anchored / drifted), then orphans — preserving the
    // order the v=4 split implied so a consumer that does not branch on
    // `status` still reads the more relevant notes first.
    let notes: Vec<NoteView> = result
        .matched
        .iter()
        .chain(result.orphaned.iter())
        .map(|n| note_view(n, explain))
        .collect();
    let data = QueryData {
        query: QuerySpecView {
            file: file.to_string_lossy().into_owned(),
            at: at.unwrap_or(""),
        },
        notes,
        malformed: result.malformed.iter().map(malformed_view).collect(),
        warnings,
    };
    json_envelope::print_success(&data)
}

fn run_text(file: &Path, at: Option<&str>, explain: bool) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let spec =
        LineSpec::from_str(at.unwrap_or("")).map_err(|e| CommandError::Usage(e.to_string()))?;
    let result = ynotes::query(&store, file, spec)?;
    render_text(file, &result, explain)
}

fn render_text(file: &Path, result: &QueryResult, explain: bool) -> Result<(), CommandError> {
    let mut out = std::io::stdout().lock();
    for rn in result.matched.iter().chain(&result.orphaned) {
        write_text_note(&mut out, rn, explain)?;
    }
    for m in &result.malformed {
        write_text_malformed(&mut out, m)?;
    }
    if result.matched.is_empty() && result.orphaned.is_empty() && result.malformed.is_empty() {
        // Finding nothing is exit 0, not an error (the query contract): the
        // "nothing here" advisory goes to stderr to keep stdout pure. A
        // nonexistent path is named explicitly — almost always a typo, so it
        // should fail loudly rather than masquerade as a real file with no
        // notes. The `warning:` segment distinguishes these from the
        // error-form `ynotes: <err>` line `main` prints; the JSON path
        // separates them via the `warnings[]` array instead.
        if file.exists() {
            eprintln!("ynotes: warning: no notes for that query");
        } else {
            eprintln!(
                "ynotes: warning: `{}` does not exist (and has no notes) — check the path",
                file.display()
            );
        }
    }
    Ok(())
}
