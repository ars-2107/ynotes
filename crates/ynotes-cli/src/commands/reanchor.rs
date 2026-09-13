//! `ynotes reanchor`, the deliberate, auditable selector-refresh pass.
//!
//! All policy lives in the engine ([`ynotes::reanchor`]); this only renders
//! the report and chooses the exit-neutral wording for a dry run.

use std::io::Write as _;

use ynotes::Store;
use ynotes::contract::ReanchorView;

use super::{json_envelope, render};
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// The review remedy appended to a re-anchored or relocated line.
///
/// Naming the id is the point: `reanchor` is the only place that learns which
/// notes need confirming, and without it the user has to correlate `list
/// --json` by target and range to act on the advice.
///
/// A dry run has written nothing, so `new_id` is merely prospective, printing
/// it would offer an id the store does not yet contain. The pass is still
/// worth flagging, so the remedy degrades to the reason alone.
fn review_remedy(review_required: bool, dry_run: bool, new_id: &str) -> String {
    if !review_required {
        String::new()
    } else if dry_run {
        " (review required once re-anchored)".to_owned()
    } else {
        format!(
            " (review required; run `ynotes confirm {}` after verification)",
            render::short_id(new_id),
        )
    }
}

/// Run (or, with `dry_run`, preview) the re-anchor pass.
///
/// # Errors
///
/// [`CommandError::Engine`] if the store cannot be read or a refreshed note
/// cannot be written. Under `--json`, any failure is rendered as the
/// agent-contract failure envelope on stdout and returned as
/// [`CommandError::Rendered`].
pub(crate) fn run(dry_run: bool, json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_json(dry_run))
    } else {
        run_text(dry_run)
    }
}

fn run_text(dry_run: bool) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let report = ynotes::reanchor(&store, dry_run).map_err(CommandError::Engine)?;

    let mut out = std::io::stdout().lock();
    let verb = if report.dry_run {
        "would re-anchor"
    } else {
        "re-anchored"
    };

    for c in &report.changed {
        let review = review_remedy(c.review_required, report.dry_run, &c.new_id);
        if c.from == c.to {
            // The note's selectors were refreshed but its line range did not
            // move: an `X:Y -> X:Y` arrow would read as a confusing no-op, so
            // state plainly that only the selectors changed.
            writeln!(
                out,
                "{} {} {}:{} (selectors updated, range unchanged){}",
                paint(verb, Colour::Yellow),
                c.target,
                c.from.start(),
                c.from.end(),
                review,
            )?;
        } else {
            writeln!(
                out,
                "{} {} {}:{} -> {}:{}{}",
                paint(verb, Colour::Yellow),
                c.target,
                c.from.start(),
                c.from.end(),
                c.to.start(),
                c.to.end(),
                review,
            )?;
        }
    }
    let reloc_verb = if report.dry_run {
        "would relocate"
    } else {
        "relocated"
    };
    for r in &report.relocated {
        let review = review_remedy(r.review_required, report.dry_run, &r.new_id);
        writeln!(
            out,
            "{} {} -> {} {}:{} -> {}:{}{}",
            paint(reloc_verb, Colour::Cyan),
            r.from_target,
            r.to_target,
            r.from.start(),
            r.from.end(),
            r.to.start(),
            r.to.end(),
            review,
        )?;
    }
    for s in &report.skipped {
        writeln!(
            out,
            "{} {} ({})",
            paint("skipped", Colour::Dim),
            s.target,
            s.reason.human(),
        )?;
    }
    // These notes needed no selector work, so nothing above mentions them, yet
    // they are exactly the ones still reading `drifted`. A dry run prints them
    // too: the outstanding review is a fact about the store, not about whether
    // this pass wrote anything.
    for p in &report.review_pending {
        writeln!(
            out,
            "{} {} (already anchored; run `ynotes confirm {}` after verification)",
            paint("review required", Colour::Yellow),
            p.target,
            render::short_id(&p.id),
        )?;
    }
    // Only mention pending reviews when there are some: the tally is read at a
    // glance on every run, and a standing ", 0 awaiting review" would be noise
    // in the overwhelmingly common clean case.
    let awaiting = if report.review_pending.is_empty() {
        String::new()
    } else {
        format!(", {} awaiting review", report.review_pending.len())
    };
    writeln!(
        out,
        "{}",
        paint(
            &format!(
                "{} {}, {} {}, {} skipped, {} already current{}{}",
                report.changed.len(),
                verb,
                report.relocated.len(),
                reloc_verb,
                report.skipped.len(),
                report.unchanged,
                awaiting,
                if report.dry_run {
                    " (dry run, nothing written)"
                } else {
                    ""
                },
            ),
            Colour::Dim,
        )
    )?;
    Ok(())
}

fn run_json(dry_run: bool) -> Result<(), CommandError> {
    let store = Store::discover().map_err(CommandError::Engine)?;
    let report = ynotes::reanchor(&store, dry_run).map_err(CommandError::Engine)?;
    json_envelope::print_success(&ReanchorView::from(&report))
}
