//! `ynotes list` — every note in the store with its current anchor status.
//!
//! A read-only inventory: same resolution as `query`, no interval filter.

use std::path::Path;

use ynotes::contract::{ListCountData, ListData, count_view, malformed_view, note_view};
use ynotes::{ResolvedNote, Store};

use super::json_envelope;
use super::render::{write_count_line, write_text_malformed, write_text_note};
use crate::command_error::CommandError;

/// List notes (optionally restricted to `file`).
///
/// # Errors
///
/// [`CommandError::Engine`] if the store or a note cannot be read. Under
/// `--json`, any failure is rendered as the agent-contract failure envelope
/// on stdout and returned as [`CommandError::Rendered`].
pub(crate) fn run(
    file: Option<&Path>,
    json: bool,
    explain: bool,
    count: bool,
) -> Result<(), CommandError> {
    match (json, count) {
        (true, true) => json_envelope::wrap(|| run_count_json(file)),
        (true, false) => json_envelope::wrap(|| run_json(file, explain)),
        (false, true) => run_count_text(file),
        (false, false) => run_text(file, explain),
    }
}

fn run_json(file: Option<&Path>, explain: bool) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let result = ynotes::list(&store, file)?;
    let warnings = list_warnings(file, &result.notes);
    let data = ListData {
        notes: result.notes.iter().map(|n| note_view(n, explain)).collect(),
        malformed: result.malformed.iter().map(malformed_view).collect(),
        warnings,
    };
    json_envelope::print_success(&data)
}

fn run_text(file: Option<&Path>, explain: bool) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let result = ynotes::list(&store, file)?;
    let mut out = std::io::stdout().lock();
    if result.notes.is_empty() && result.malformed.is_empty() {
        match file {
            Some(p) => eprintln!("ynotes: {}", empty_filter_message(p)),
            None => eprintln!("ynotes: no notes in this store"),
        }
    } else {
        for rn in &result.notes {
            write_text_note(&mut out, rn, explain)?;
        }
        for m in &result.malformed {
            write_text_malformed(&mut out, m)?;
        }
    }
    Ok(())
}

fn run_count_json(file: Option<&Path>) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let result = ynotes::list(&store, file)?;
    let warnings = list_warnings(file, &result.notes);
    let count = count_view(result.notes.iter());
    let data = ListCountData {
        count,
        malformed: result.malformed.len(),
        warnings,
    };
    json_envelope::print_success(&data)
}

fn run_count_text(file: Option<&Path>) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let result = ynotes::list(&store, file)?;
    let count = count_view(result.notes.iter());
    let mut out = std::io::stdout().lock();
    write_count_line(&mut out, &count, result.malformed.len())
}

/// Advisories for the filter-yielded-nothing cases — exactly the cases where
/// an empty `notes` array would be ambiguous to a machine consumer (was the
/// filter wrong? did the store empty out under it?). With no filter, an
/// empty array is self-explanatory and `warnings` stays empty.
fn list_warnings(file: Option<&Path>, notes: &[ResolvedNote]) -> Vec<String> {
    if !notes.is_empty() {
        return Vec::new();
    }
    file.map(|p| vec![empty_filter_message(p)])
        .unwrap_or_default()
}

/// The single source of truth for the "filter matched nothing" message —
/// shared by the human form (above) and the `warnings` array, so the two can
/// never disagree about what happened. Intent is read as *directory* when the
/// path is a real directory *or* the user wrote a trailing separator (`dir/`),
/// since that is the natural way to express "I meant a directory" for a path
/// that does not exist yet. The suffix names exactly what is true about the
/// path — that it does not exist, or that it does but holds no notes — rather
/// than the generic `(check the path)`.
fn empty_filter_message(p: &Path) -> String {
    let dir_intent = p.is_dir() || ends_with_separator(p);
    let exists = p.exists();
    match (dir_intent, exists) {
        (true, true) => format!("no notes under `{}`", p.display()),
        (true, false) => format!("no notes under `{}` (no such directory)", p.display()),
        (false, true) => format!("no notes for `{}`", p.display()),
        (false, false) => format!("no notes for `{}` (no such file)", p.display()),
    }
}

/// Whether the user's literal path text ended with a path separator — read as
/// "I meant a directory" intent for paths that may not exist yet. Inspects the
/// raw trailing byte of the [`OsStr`](std::ffi::OsStr) (ASCII separators
/// survive every platform's path encoding), so no lossy UTF-8 conversion is
/// needed. `/` is honoured everywhere; `\` only on Windows, where users may
/// type either form in a CLI argument.
fn ends_with_separator(p: &Path) -> bool {
    p.as_os_str()
        .as_encoded_bytes()
        .last()
        .is_some_and(|&b| b == b'/' || (cfg!(windows) && b == b'\\'))
}
