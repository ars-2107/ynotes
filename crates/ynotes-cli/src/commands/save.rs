//! `ynotes save` — capture a note's selector bundle and persist it.
//!
//! The location form is the scope: no location is a file note, `N` a line
//! note, `A:B` a range note. The bundle is captured now so the note can
//! re-anchor later even as the file is edited outside ynotes.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::str::FromStr as _;

use ynotes::contract::{SaveData, scope_word_json};
use ynotes::{
    LineSpec, Note, Scope, SelectorBundle, SourceFile, Store, resolve_scope, resolve_scope_verified,
};

use super::json_envelope;
use super::safety::reject_if_escapes_workdir;
use crate::command_error::CommandError;

/// Capture and store a note.
///
/// # Errors
///
/// [`CommandError::Usage`] for a bad location, an empty file note, an empty
/// message, or a `--code` quote the engine could not verify (found nowhere,
/// or ambiguous); [`CommandError::Engine`] / [`CommandError::Io`] otherwise.
/// Under `--json`, any failure is rendered as the agent-contract failure
/// envelope on stdout and returned as [`CommandError::Rendered`].
pub(crate) fn run(
    file: &Path,
    at: Option<&str>,
    code: Option<&str>,
    message: Option<&str>,
    json: bool,
) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_inner(file, at, code, message, true))
    } else {
        run_inner(file, at, code, message, false)
    }
}

fn run_inner(
    file: &Path,
    at: Option<&str>,
    code: Option<&str>,
    message: Option<&str>,
    json: bool,
) -> Result<(), CommandError> {
    // Discover from the current directory, exactly as every other subcommand
    // does. The start point must be the cwd, not the target file's parent: a
    // target *outside* the store has to be discovered against the current store
    // and refused by `relativize` below (a usage error, exit 2) — starting from
    // the target's own parent would instead send discovery off to wherever that
    // path lives and lose the store entirely. When no store is found,
    // `discover_or_init_from` bootstraps one at the enclosing git root — the
    // first-save zero-ceremony path.
    let cwd = std::env::current_dir()?;
    let (store, store_created) =
        Store::discover_or_init_from(&cwd).map_err(CommandError::Engine)?;
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

    // Bare `?` on both arms so the engine's typed usage errors — including
    // `CodeNotFound`/`CodeAmbiguous` from the verified path — reach the
    // `From<ynotes::Error>` classification and exit `2`.
    let (scope, range) = match code {
        Some(code) => resolve_scope_verified(spec, code, &source)?,
        None => resolve_scope(spec, &source)?,
    };

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
        json_envelope::print_success(&SaveData {
            id: note.id.clone(),
            target: target.clone(),
            scope: scope_word_json(scope).to_owned(),
            range: [range.start(), range.end()],
            created,
            store_created,
        })?;
    } else {
        let mut out = std::io::stdout().lock();
        let superseded_clause = if superseded > 0 {
            format!(" (superseded {superseded} earlier note(s) here)")
        } else {
            String::new()
        };
        // Announce the zero-ceremony bootstrap on the human line: the first save
        // in a git repo just created the store, so name where it landed. The
        // `--json` branch above carries the same signal as `store_created`.
        let created_store_clause = if store_created {
            format!(
                " (created .ynotes store at {})",
                store.workdir().map_err(CommandError::Engine)?.display()
            )
        } else {
            String::new()
        };
        writeln!(
            out,
            "{} {} note {} for {} {}:{}{}{}",
            if created { "saved" } else { "exists" },
            scope_word(scope),
            &note.id[..note.id.len().min(12)],
            target,
            range.start(),
            range.end(),
            superseded_clause,
            created_store_clause
        )?;
    }
    Ok(())
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
