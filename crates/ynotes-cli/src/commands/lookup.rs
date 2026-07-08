//! `ynotes lookup` — resolve a stable `(target, body)` handle to live note id(s).
//!
//! A read-only finder. The id is content-addressed over the anchor, so it
//! rotates on `reanchor`/`update`; this command re-derives the current id(s)
//! from the parts that persist (target path + body text). Text output is
//! id-first so the id is copyable into a PR/comment; `--json` reuses the shared
//! note view.

use std::io::Write as _;
use std::path::Path;

use serde::Serialize;
use ynotes::Store;

use super::json_envelope;
use super::render::{NoteView, note_view};
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Run the lookup.
///
/// # Errors
///
/// [`CommandError::Usage`] when neither `--target` nor `--body-contains` is
/// given. [`CommandError::Engine`] if the store or a note cannot be read. Under
/// `--json`, any failure is rendered as the agent-contract failure envelope and
/// returned as [`CommandError::Rendered`].
pub(crate) fn run(
    target: Option<&Path>,
    body_contains: Option<&str>,
    json: bool,
) -> Result<(), CommandError> {
    if target.is_none() && body_contains.is_none() {
        return Err(CommandError::Usage(
            "lookup needs at least one of --target or --body-contains".to_owned(),
        ));
    }
    if json {
        json_envelope::wrap(|| run_json(target, body_contains))
    } else {
        run_text(target, body_contains)
    }
}

#[derive(Serialize)]
struct LookupData {
    notes: Vec<NoteView>,
}

fn run_json(target: Option<&Path>, body_contains: Option<&str>) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    // `lookup` matches on (target, body); a malformed record cannot be read to
    // match either, so it structurally cannot appear here — `query`/`list`/
    // `doctor` are where a corrupt record surfaces. The payload stays `{notes}`.
    let notes = ynotes::lookup(&store, target, body_contains)?.notes;
    let data = LookupData {
        notes: notes.iter().map(|n| note_view(n, false)).collect(),
    };
    json_envelope::print_success(&data)
}

fn run_text(target: Option<&Path>, body_contains: Option<&str>) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let notes = ynotes::lookup(&store, target, body_contains)?.notes;
    if notes.is_empty() {
        eprintln!("ynotes: no notes match");
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for rn in &notes {
        let v = note_view(rn, false);
        let range = v
            .resolved_range
            .map_or_else(|| "?".to_owned(), |[a, b]| format!("{a}:{b}"));
        // id-first so it can be copied into a PR/comment.
        writeln!(
            out,
            "{} {} {}:{}  [{}]",
            v.id,
            v.target,
            v.scope,
            range,
            paint(v.status, status_colour(v.status)),
        )?;
        for line in v.body.lines() {
            writeln!(out, "  {}", paint(line, Colour::Dim))?;
        }
    }
    Ok(())
}

/// Colour a status word the same way `list`/`query` do.
fn status_colour(status: &str) -> Colour {
    match status {
        "anchored" => Colour::Green,
        "drifted" => Colour::Yellow,
        _ => Colour::Red,
    }
}
