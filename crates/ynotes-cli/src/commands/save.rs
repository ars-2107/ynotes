//! `ynotes save` — capture a note's selector bundle and persist it.
//!
//! The location form is the scope: no location is a file note, `N` a line
//! note, `A:B` a range note. The bundle is captured now so the note can
//! re-anchor later even as the file is edited outside ynotes.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::str::FromStr as _;

use serde::Serialize;
use ynotes::{LineRange, LineSpec, Note, Scope, SelectorBundle, SourceFile, Store};

use super::json_envelope;
use super::safety::reject_if_escapes_workdir;
use crate::command_error::CommandError;

/// Capture and store a note.
///
/// # Errors
///
/// [`CommandError::Usage`] for a bad location, an empty file note, or an
/// empty message; [`CommandError::Engine`] / [`CommandError::Io`] otherwise.
/// Under `--json`, any failure is rendered as the agent-contract failure
/// envelope on stdout and returned as [`CommandError::Rendered`].
pub(crate) fn run(
    file: &Path,
    at: Option<&str>,
    message: Option<&str>,
    json: bool,
) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_inner(file, at, message, true))
    } else {
        run_inner(file, at, message, false)
    }
}

fn run_inner(
    file: &Path,
    at: Option<&str>,
    message: Option<&str>,
    json: bool,
) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    // The symlink-escape guard runs before `relativize` because it answers a
    // question lexical relativisation cannot: a path *spelled* inside the
    // work tree but pointing outside via a symlink. It now restricts itself
    // to actual symlinks, so a non-symlink path that sits outside the work
    // tree (an absolute outside path, a device like `/dev/null`) falls
    // through to `relativize` and surfaces as a single, accurate
    // `OutsideStore` — never blamed on a symlink that is not there.
    reject_if_escapes_workdir(file, &store)?;
    // `?`, not `.map_err(CommandError::Engine)`, so the engine's typed
    // `OutsideStore` lands as a usage error (exit `2`) — same classification
    // as the symlink-escape guard, so both kinds of "outside the store"
    // exit identically.
    let target = store.relativize(file)?;

    let source = SourceFile::read(file).map_err(CommandError::Engine)?;
    let spec =
        LineSpec::from_str(at.unwrap_or("")).map_err(|e| CommandError::Usage(e.to_string()))?;

    let (scope, range) = resolve_scope(spec, &source)?;

    let body = match message {
        Some(m) => m.to_owned(),
        // A piped body arrives with the shell/heredoc's trailing newline(s); a
        // `-m` body does not. Trim them so the two paths store the same text
        // (and hash to the same content-addressed id for identical content).
        None => read_stdin()?.trim_end().to_owned(),
    };
    if body.trim().is_empty() {
        return Err(CommandError::Usage(
            "no note body provided — pass `-m \"<text>\"` or pipe text on stdin".to_owned(),
        ));
    }

    let bundle = SelectorBundle::capture_full(&source, range).map_err(CommandError::Engine)?;

    let note = Note::new(target.clone(), scope, bundle, body).map_err(CommandError::Engine)?;
    // `save_superseding`, not `save`: editing a note at the same location must
    // replace it, not leave the old body coexisting under its own content id.
    let (created, superseded) = store
        .save_superseding(&note)
        .map_err(CommandError::Engine)?;

    if json {
        // A stable identity record for an agent to capture and reference
        // later. Serialised with `serde_json`, never string interpolation:
        // the `--json` surface is an agent contract and must stay valid JSON
        // for any target path (a quote in a filename must be escaped).
        #[derive(Serialize)]
        struct SaveData<'a> {
            id: &'a str,
            target: &'a str,
            scope: &'a str,
            range: [u32; 2],
            created: bool,
        }
        json_envelope::print_success(&SaveData {
            id: &note.id,
            target: &target,
            scope: super::render::scope_word_json(scope),
            range: [range.start(), range.end()],
            created,
        })?;
    } else {
        let mut out = std::io::stdout().lock();
        let superseded_clause = if superseded > 0 {
            format!(" (superseded {superseded} earlier note(s) here)")
        } else {
            String::new()
        };
        writeln!(
            out,
            "{} {} note {} for {} {}:{}{}",
            if created { "saved" } else { "exists" },
            scope_word(scope),
            &note.id[..note.id.len().min(12)],
            target,
            range.start(),
            range.end(),
            superseded_clause
        )?;
    }
    Ok(())
}

/// Maps the parsed location to a scope and the range to capture. A file note
/// captures the whole file; an empty file cannot be noted.
///
/// A range that runs past the last line is rejected as a [`CommandError::Usage`]
/// (exit `2`), the same class as line `0`: both are bad locations the user
/// asked for, not runtime failures. Without this check, a beyond-EOF range
/// would only fail later inside the engine and exit `1` — inconsistent.
fn resolve_scope(spec: LineSpec, source: &SourceFile) -> Result<(Scope, LineRange), CommandError> {
    let usage = |m: String| CommandError::Usage(m);
    let (scope, range) = match spec {
        LineSpec::Whole => {
            let n = source.line_count();
            if n == 0 {
                return Err(usage("cannot save a note for an empty file".into()));
            }
            let r = LineRange::new(1, n).map_err(|e| usage(e.to_string()))?;
            (Scope::File, r)
        }
        LineSpec::Line(n) => {
            let r = LineRange::new(n, n).map_err(|e| usage(e.to_string()))?;
            (Scope::Line, r)
        }
        LineSpec::Range(r) => (Scope::Range, r),
    };

    let lines = source.line_count();
    if range.end() > lines {
        return Err(usage(format!(
            "line {} is past the end of the file ({lines} lines)",
            range.end()
        )));
    }
    Ok((scope, range))
}

/// Reads the whole of stdin as the note body.
fn read_stdin() -> Result<String, CommandError> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

/// Human word for a scope, for the confirmation line.
fn scope_word(scope: Scope) -> &'static str {
    match scope {
        Scope::Line => "line",
        Scope::Range => "range",
        Scope::File => "file",
    }
}
