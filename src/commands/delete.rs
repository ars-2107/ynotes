//! `ynotes delete` — remove one or more notes from the store, by id or hex
//! prefix.
//!
//! Selection accepts a short hex prefix: an unambiguous hex prefix
//! is enough, but it must uniquely identify a single note. Ambiguous and
//! missing prefixes are *named* in the report rather than silently dropped,
//! and successful deletions still happen even when other ids in the same
//! call fail — only the exit code (`1` on any failure) carries the global
//! signal.

use std::io::Write as _;

use serde::Serialize;
use ynotes::Store;

use super::id::validate_prefix;
use super::json_envelope;
use super::render::{body_excerpt, scope_word_json, short_id};
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Run the delete pass.
///
/// # Errors
///
/// [`CommandError::Usage`] when any id argument is malformed (non-hex or
/// shorter than the shared minimum prefix) or empty. [`CommandError::Engine`]
/// / [`CommandError::Io`] if the store cannot be read or a removal cannot be
/// written. Under `--json`, any failure is rendered as the agent-contract
/// failure envelope and returned as [`CommandError::Rendered`].
pub(crate) fn run(ids: &[String], dry_run: bool, json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_inner(ids, dry_run, true))
    } else {
        run_inner(ids, dry_run, false)
    }
}

fn run_inner(ids: &[String], dry_run: bool, json: bool) -> Result<(), CommandError> {
    if ids.is_empty() {
        return Err(CommandError::Usage(
            "delete requires at least one note id".to_owned(),
        ));
    }
    for id in ids {
        validate_prefix(id)?;
    }

    let store = Store::discover()?;

    let mut deleted: Vec<DeletedView> = Vec::new();
    let mut deleted_unreadable: Vec<String> = Vec::new();
    let mut ambiguous: Vec<AmbiguousView> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for requested in ids {
        let matches = store.find_by_id_prefix(requested)?;
        match matches.as_slice() {
            // No *readable* match. The one removable corrupt case is an exact
            // id naming an unreadable record on disk: `purge_unreadable` is
            // guarded to the exact id (a prefix names no record file) and
            // refuses a readable note, so this never bypasses the preview path
            // above. Anything else stays `not_found`.
            [] => {
                if store.purge_unreadable(requested, dry_run)? {
                    deleted_unreadable.push(requested.clone());
                } else {
                    not_found.push(requested.clone());
                }
            }
            [only] => {
                let view = DeletedView {
                    requested: requested.clone(),
                    id: only.id.clone(),
                    target: only.target.clone(),
                    scope: scope_word_json(only.scope),
                    range: [
                        only.bundle.position.range.start(),
                        only.bundle.position.range.end(),
                    ],
                    body_excerpt: body_excerpt(&only.body),
                };
                if !dry_run {
                    store.remove(only)?;
                }
                deleted.push(view);
            }
            many => ambiguous.push(AmbiguousView {
                requested: requested.clone(),
                candidates: many.iter().map(|n| n.id.clone()).collect(),
            }),
        }
    }

    let had_failure = !ambiguous.is_empty() || !not_found.is_empty();

    if json {
        let data = DeleteData {
            requested: ids.len(),
            deleted,
            deleted_unreadable,
            ambiguous,
            not_found,
            dry_run,
        };
        json_envelope::print_success(&data)?;
    } else {
        write_text(
            &deleted,
            &deleted_unreadable,
            &ambiguous,
            &not_found,
            dry_run,
        )?;
    }

    if had_failure {
        // Successful deletions are already committed and named in the report;
        // exit non-zero only to signal that some requested id was unresolvable.
        Err(CommandError::Rendered {
            code: crate::command_error::codes::FAILURE,
        })
    } else {
        Ok(())
    }
}

/// Stable JSON payload for `delete --json`.
///
/// `requested` is the count of ids the caller passed; the four category arrays
/// partition the outcome (each requested id appears in exactly one). Always
/// emitted in full — empty arrays included — so a consumer never has to branch
/// on key presence. `deleted_unreadable` (added in `v9`) holds the ids of
/// *corrupt* records removed by exact id: they cannot be read to supply a
/// target/scope/body, so they are listed as bare ids rather than as rich
/// `deleted[]` entries. Unlike `ambiguous`/`not_found`, an entry here is a
/// success, not a failure, and does not flip the exit code.
#[derive(Serialize)]
struct DeleteData {
    requested: usize,
    deleted: Vec<DeletedView>,
    deleted_unreadable: Vec<String>,
    ambiguous: Vec<AmbiguousView>,
    not_found: Vec<String>,
    dry_run: bool,
}

#[derive(Serialize)]
struct DeletedView {
    requested: String,
    id: String,
    target: String,
    scope: &'static str,
    range: [u32; 2],
    body_excerpt: String,
}

#[derive(Serialize)]
struct AmbiguousView {
    requested: String,
    candidates: Vec<String>,
}

fn write_text(
    deleted: &[DeletedView],
    deleted_unreadable: &[String],
    ambiguous: &[AmbiguousView],
    not_found: &[String],
    dry_run: bool,
) -> Result<(), CommandError> {
    let mut out = std::io::stdout().lock();
    let verb = if dry_run { "would delete" } else { "deleted" };

    for d in deleted {
        writeln!(
            out,
            "{} {} {} {}:{}:{}",
            paint(verb, Colour::Yellow),
            short_id(&d.id),
            d.target,
            d.scope,
            d.range[0],
            d.range[1],
        )?;
        if !d.body_excerpt.is_empty() {
            writeln!(out, "  {}", paint(&d.body_excerpt, Colour::Dim))?;
        }
    }
    // A purged corrupt record has no body/target/scope to show — just name the
    // id and flag why it had no preview.
    for id in deleted_unreadable {
        writeln!(
            out,
            "{} {} (unreadable record)",
            paint(verb, Colour::Yellow),
            short_id(id),
        )?;
    }
    for a in ambiguous {
        writeln!(
            out,
            "{} {} ({} candidates)",
            paint("ambiguous", Colour::Red),
            a.requested,
            a.candidates.len(),
        )?;
        for c in &a.candidates {
            writeln!(out, "  {}", paint(&short_id(c), Colour::Dim))?;
        }
    }
    for nf in not_found {
        writeln!(out, "{} {}", paint("no match", Colour::Red), nf)?;
    }

    let n = deleted.len();
    let u = deleted_unreadable.len();
    let amb = ambiguous.len();
    let nf = not_found.len();
    let suffix = if dry_run {
        " (dry run — nothing written)"
    } else {
        ""
    };
    // Only name the unreadable count when there is one, so a routine delete's
    // summary line is unchanged.
    let unreadable = if u > 0 {
        format!(", {u} unreadable purged")
    } else {
        String::new()
    };
    writeln!(
        out,
        "{}",
        paint(
            &format!("{n} {verb}{unreadable}, {amb} ambiguous, {nf} not found{suffix}"),
            Colour::Dim,
        )
    )?;
    Ok(())
}
