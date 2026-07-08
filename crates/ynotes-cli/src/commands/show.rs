//! `ynotes show` — view one note by id (or hex prefix), resolved against
//! current code.
//!
//! The by-id read: `query` selects by file and line, `list` walks the whole
//! store; this resolves exactly one note against its stored target and renders
//! it with the shared note view. Read-only (invariant #8). Rename-following is
//! deliberately out of scope — a note whose file was renamed away resolves
//! `orphaned` here (its code is not at the old path); `query` on the new path is
//! the rename-aware read.

use ynotes::contract::{ShowData, note_view};
use ynotes::{GitContext, ResolvedNote, SourceFile, Store, resolve};

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

/// Resolve the id/prefix to one note and anchor it against its stored target,
/// mirroring how `update` re-resolves a note before rewriting it. Returns the
/// resolved note and any advisories (a missing target file).
fn resolve_one(id: &str) -> Result<(ResolvedNote, Vec<String>), CommandError> {
    validate_prefix(id)?;
    let store = Store::discover()?;
    let note = resolve_unique(&store, id)?;

    let workdir = store.workdir()?.to_path_buf();
    let abs = workdir.join(&note.target);
    // Missing/unreadable target => empty source => the note orphans and is still
    // shown (never a hard failure), exactly as `list` resolves a note.
    let source =
        SourceFile::read(&abs).unwrap_or_else(|_| SourceFile::from_lines(abs.clone(), Vec::new()));
    let git = GitContext::discover(&workdir);
    let resolution = resolve(&note.bundle, &source, git.as_ref());

    let mut warnings = Vec::new();
    if !abs.exists() {
        warnings.push(format!(
            "`{}` does not exist (the note's target file is gone)",
            note.target
        ));
    }

    Ok((
        ResolvedNote {
            note,
            resolution,
            relocated_from: None,
        },
        warnings,
    ))
}
