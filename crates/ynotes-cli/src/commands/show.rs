//! Render one note selected by ID or unambiguous prefix.
//! [`ynotes::resolve_one`] provides the same rename-aware resolution as inventory.

use ynotes::contract::{ShowData, note_view};
use ynotes::{ResolvedNote, Store};

use super::id::{resolve_unique, validate_prefix};
use super::json_envelope;
use super::render::write_text_note;
use crate::command_error::CommandError;

/// Show the note identified by `id` (a full id or unambiguous hex prefix).
///
/// # Errors
///
/// [`CommandError::Usage`] if the id is malformed, the prefix is ambiguous, or
/// it matches no note; [`CommandError::Engine`] / [`CommandError::Io`]
/// otherwise. Under `--json`, any failure is rendered as the agent-contract
/// failure envelope on stdout and returned as [`CommandError::Rendered`].
pub(crate) fn run(id: &str, json: bool, explain: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_json(id, explain))
    } else {
        run_text(id, explain)
    }
}

fn run_json(id: &str, explain: bool) -> Result<(), CommandError> {
    let (resolved, warnings) = resolve_one(id)?;
    let data = ShowData {
        note: note_view(&resolved, explain),
        warnings,
    };
    json_envelope::print_success(&data)
}

fn run_text(id: &str, explain: bool) -> Result<(), CommandError> {
    let (resolved, _warnings) = resolve_one(id)?;
    let mut out = std::io::stdout().lock();
    // The note carries its own status (`orphaned ✗ last seen …` when the target
    // is gone), so the missing-file advisory is redundant in the human form and
    // stays out of it; it is surfaced to a machine consumer via `warnings`.
    write_text_note(&mut out, &resolved, explain)
}

/// Resolve the id/prefix to one note through the engine's shared single-note
/// resolution, so this read cannot disagree with `list`, `query`, or the MCP
/// `notes` tool about the same note. Returns it with any advisories.
fn resolve_one(id: &str) -> Result<(ResolvedNote, Vec<String>), CommandError> {
    validate_prefix(id)?;
    let store = Store::discover()?;
    let note = resolve_unique(&store, id)?;

    let abs = store.workdir()?.join(&note.target);
    // Missing/unreadable target => empty source => the note orphans and is
    // still shown, never a hard failure (invariant #4).
    let resolved = ynotes::resolve_one(&store, note)?;

    let mut warnings = Vec::new();
    // A renamed file is absent at its stored target yet plainly present at the
    // new one, so this advisory would be actively wrong there.
    if !abs.exists() && resolved.relocated_to.is_none() {
        warnings.push(format!(
            "`{}` does not exist (the note's target file is gone)",
            resolved.note.target
        ));
    }

    Ok((resolved, warnings))
}
