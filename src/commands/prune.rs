//! `ynotes prune` — remove every orphaned note from the store.
//!
//! An orphan is a note no content rung could locate — its code is gone or
//! wholly rewritten. ynotes never auto-prunes (an orphan may still be
//! historically valuable, and its target may be temporarily missing rather
//! than permanently lost), so this is the deliberate cleanup pass. Notes
//! that are merely `drifted` are *not* touched: `reanchor` is for them.

use std::io::Write as _;

use serde::Serialize;
use ynotes::{AnchorStatus, Store};

use super::json_envelope;
use super::render::{body_excerpt, scope_word_json, short_id};
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Run (or, with `dry_run`, preview) the prune pass.
///
/// # Errors
///
/// [`CommandError::Engine`] / [`CommandError::Io`] if the store cannot be
/// read or a removal cannot be written. Under `--json`, any failure is
/// rendered as the agent-contract failure envelope on stdout and returned
/// as [`CommandError::Rendered`].
pub(crate) fn run(dry_run: bool, json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_inner(dry_run, true))
    } else {
        run_inner(dry_run, false)
    }
}

fn run_inner(dry_run: bool, json: bool) -> Result<(), CommandError> {
    let store = Store::discover()?;
    let resolved = ynotes::list(&store, None)?;

    let total = resolved.len();
    let mut pruned: Vec<PrunedView> = Vec::new();

    for rn in &resolved {
        let AnchorStatus::Orphaned { last_known } = rn.resolution.status else {
            continue;
        };
        if !dry_run {
            store.remove(&rn.note)?;
        }
        pruned.push(PrunedView {
            id: rn.note.id.clone(),
            target: rn.note.target.clone(),
            scope: scope_word_json(rn.note.scope),
            last_known: [last_known.start(), last_known.end()],
            body_excerpt: body_excerpt(&rn.note.body),
        });
    }

    let kept = total - pruned.len();

    if json {
        let data = PruneData {
            pruned,
            kept,
            dry_run,
        };
        json_envelope::print_success(&data)?;
    } else {
        write_text(&pruned, kept, dry_run)?;
    }
    Ok(())
}

/// Stable JSON payload for `prune --json`. `pruned` carries one entry per
/// note removed (or that *would* be removed under `--dry-run`); `kept` is
/// the count left in the store; `dry_run` echoes the flag so a consumer
/// never has to infer it from the call.
#[derive(Serialize)]
struct PruneData {
    pruned: Vec<PrunedView>,
    kept: usize,
    dry_run: bool,
}

#[derive(Serialize)]
struct PrunedView {
    id: String,
    target: String,
    scope: &'static str,
    last_known: [u32; 2],
    body_excerpt: String,
}

fn write_text(pruned: &[PrunedView], kept: usize, dry_run: bool) -> Result<(), CommandError> {
    let mut out = std::io::stdout().lock();
    let verb = if dry_run { "would prune" } else { "pruned" };

    for p in pruned {
        writeln!(
            out,
            "{} {} {} {}:{}",
            paint(verb, Colour::Yellow),
            short_id(&p.id),
            p.target,
            p.last_known[0],
            p.last_known[1],
        )?;
        if !p.body_excerpt.is_empty() {
            writeln!(out, "  {}", paint(&p.body_excerpt, Colour::Dim))?;
        }
    }
    let suffix = if dry_run {
        " (dry run — nothing written)"
    } else {
        ""
    };
    writeln!(
        out,
        "{}",
        paint(
            &format!("{} {verb}, {kept} kept{suffix}", pruned.len()),
            Colour::Dim,
        )
    )?;
    Ok(())
}
