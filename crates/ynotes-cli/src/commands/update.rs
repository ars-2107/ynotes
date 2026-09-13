//! Replace a note body at its currently resolved region.
//! Recapture selectors and review basis while preserving target, scope, and
//! creation time. The replacement supersedes the previous content-addressed ID.

use std::io::{Read as _, Write as _};
use std::path::Path;

use serde::Serialize;
use ynotes::contract::scope_word_json;
use ynotes::{GitContext, SelectorBundle, SourceFile, Store, resolve};

use super::id::{resolve_unique, validate_prefix};
use super::json_envelope;
use super::render::short_id;
use super::safety::reject_if_escapes_workdir;
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Run the update.
///
/// # Errors
///
/// [`CommandError::Usage`] if the id is malformed, the body is empty, or
/// the prefix is ambiguous / matches no note; [`CommandError::Engine`] /
/// [`CommandError::Io`] otherwise. Under `--json`, any failure is rendered
/// as the agent-contract failure envelope on stdout and returned as
/// [`CommandError::Rendered`].
pub(crate) fn run(id: &str, message: Option<&str>, json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_inner(id, message, true))
    } else {
        run_inner(id, message, false)
    }
}

fn run_inner(id: &str, message: Option<&str>, json: bool) -> Result<(), CommandError> {
    validate_prefix(id)?;

    let store = Store::discover()?;
    let original = resolve_unique(&store, id)?;

    // Read the new body the same way `save` does (matching `-m` vs stdin
    // semantics, including the stdin trailing-newline trim) so identical
    // content hashes to the same id whichever path delivered it.
    let body = match message {
        Some(m) => m.to_owned(),
        None => read_stdin()?.trim_end().to_owned(),
    };
    if body.trim().is_empty() {
        return Err(CommandError::Usage(
            "no note body provided, pass `-m \"<text>\"` or pipe text on stdin".to_owned(),
        ));
    }

    let workdir = store.workdir()?.to_path_buf();
    let abs = workdir.join(&original.target);
    // Save and reanchor both refuse a target whose canonical path escapes the
    // work tree, closing the post-save symlink-swap attack. Update read from
    // `abs` directly without this guard, which let a symlink dropped at
    // `<workdir>/<stored-target>` between save and update pull external
    // content into the refreshed bundle.
    reject_if_escapes_workdir(&abs, &store)?;
    let source = SourceFile::read(&abs)?;

    // Anchor the note in the current file, the same way `reanchor` does:
    // re-resolve, and fall back to the saved position when no rung located
    // it. That keeps `update` useful for a drifted note (anchors where the
    // code actually is now) while still working for an orphan whose original
    // range still fits the file.
    let git = GitContext::discover(abs.parent().unwrap_or_else(|| Path::new(".")));
    let resolution = resolve(&original.bundle, &source, git.as_ref());
    let range = resolution.range.unwrap_or(original.bundle.position.range);

    let fresh = SelectorBundle::capture_full(&source, range)?;
    let refreshed = original.clone().with_bundle_and_body(fresh, body)?;

    // Write the refreshed note before removing the old one, so a crash
    // between leaves both records (and the new id resolves) rather than a
    // hole. Mirrors `reanchor`'s safety ordering.
    let created = store.save(&refreshed)?;
    if refreshed.id != original.id {
        store.remove(&original)?;
    }

    if json {
        let data = UpdateData {
            id: &refreshed.id,
            previous_id: &original.id,
            target: &refreshed.target,
            scope: scope_word_json(refreshed.scope),
            range: [range.start(), range.end()],
            created,
        };
        json_envelope::print_success(&data)?;
    } else {
        let mut out = std::io::stdout().lock();
        let short_new = short_id(&refreshed.id);
        let short_old = short_id(&original.id);
        if refreshed.id == original.id {
            writeln!(
                out,
                "{} {} (body identical, nothing to do)",
                paint("unchanged", Colour::Dim),
                short_new,
            )?;
        } else {
            writeln!(
                out,
                "{} {} (was {}) {} {}:{}",
                paint("updated", Colour::Yellow),
                short_new,
                short_old,
                refreshed.target,
                range.start(),
                range.end(),
            )?;
        }
    }
    Ok(())
}

fn read_stdin() -> Result<String, CommandError> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

/// Stable JSON payload for `update --json`. `previous_id` exposes the
/// content-hash id the new note supersedes, so a caller that cached the old
/// id can update its reference.
#[derive(Serialize)]
struct UpdateData<'a> {
    id: &'a str,
    previous_id: &'a str,
    target: &'a str,
    scope: &'static str,
    range: [u32; 2],
    created: bool,
}
