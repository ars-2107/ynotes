//! `ynotes reanchor` — the deliberate, auditable selector-refresh pass.
//!
//! All policy lives in the engine ([`ynotes::reanchor`]); this only renders
//! the report and chooses the exit-neutral wording for a dry run.

use std::io::Write as _;

use serde::Serialize;
use ynotes::{ReanchorReport, Store};

use super::json_envelope;
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

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
        if c.from == c.to {
            // The note's selectors were refreshed but its line range did not
            // move: an `X:Y -> X:Y` arrow would read as a confusing no-op, so
            // state plainly that only the selectors changed.
            writeln!(
                out,
                "{} {} {}:{} (selectors updated, range unchanged)",
                paint(verb, Colour::Yellow),
                c.target,
                c.from.start(),
                c.from.end(),
            )?;
        } else {
            writeln!(
                out,
                "{} {} {}:{} -> {}:{}",
                paint(verb, Colour::Yellow),
                c.target,
                c.from.start(),
                c.from.end(),
                c.to.start(),
                c.to.end(),
            )?;
        }
    }
    let reloc_verb = if report.dry_run {
        "would relocate"
    } else {
        "relocated"
    };
    for r in &report.relocated {
        writeln!(
            out,
            "{} {} -> {} {}:{} -> {}:{}",
            paint(reloc_verb, Colour::Cyan),
            r.from_target,
            r.to_target,
            r.from.start(),
            r.from.end(),
            r.to.start(),
            r.to.end(),
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
    writeln!(
        out,
        "{}",
        paint(
            &format!(
                "{} {}, {} {}, {} skipped, {} already current{}",
                report.changed.len(),
                verb,
                report.relocated.len(),
                reloc_verb,
                report.skipped.len(),
                report.unchanged,
                if report.dry_run {
                    " (dry run — nothing written)"
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

/// JSON projection of [`ReanchorReport`]. Defined locally (not by deriving
/// `Serialize` on the engine type) so the agent contract stays a deliberate
/// surface a refactor inside the engine cannot accidentally widen.
#[derive(Serialize)]
struct ReanchorView<'a> {
    dry_run: bool,
    changed: Vec<ChangedView<'a>>,
    relocated: Vec<RelocatedView<'a>>,
    skipped: Vec<SkippedView<'a>>,
    unchanged: usize,
}

#[derive(Serialize)]
struct ChangedView<'a> {
    target: &'a str,
    old_id: &'a str,
    new_id: &'a str,
    from: [u32; 2],
    to: [u32; 2],
}

/// JSON projection of a note moved to a renamed file. Its own partition (not
/// folded into `changed`) because a relocation changes the note's `target` — a
/// distinct event a consumer branches on separately.
#[derive(Serialize)]
struct RelocatedView<'a> {
    from_target: &'a str,
    to_target: &'a str,
    old_id: &'a str,
    new_id: &'a str,
    from: [u32; 2],
    to: [u32; 2],
}

#[derive(Serialize)]
struct SkippedView<'a> {
    target: &'a str,
    id: &'a str,
    /// A stable, lower-kebab-case classification an agent can branch on
    /// without parsing prose; the human form lives in [`Self::reason`].
    reason_code: &'static str,
    reason: &'static str,
}

impl<'a> From<&'a ReanchorReport> for ReanchorView<'a> {
    fn from(r: &'a ReanchorReport) -> Self {
        Self {
            dry_run: r.dry_run,
            changed: r
                .changed
                .iter()
                .map(|c| ChangedView {
                    target: &c.target,
                    old_id: &c.old_id,
                    new_id: &c.new_id,
                    from: [c.from.start(), c.from.end()],
                    to: [c.to.start(), c.to.end()],
                })
                .collect(),
            relocated: r
                .relocated
                .iter()
                .map(|r| RelocatedView {
                    from_target: &r.from_target,
                    to_target: &r.to_target,
                    old_id: &r.old_id,
                    new_id: &r.new_id,
                    from: [r.from.start(), r.from.end()],
                    to: [r.to.start(), r.to.end()],
                })
                .collect(),
            skipped: r
                .skipped
                .iter()
                .map(|s| SkippedView {
                    target: &s.target,
                    id: &s.id,
                    reason_code: s.reason.code(),
                    reason: s.reason.human(),
                })
                .collect(),
            unchanged: r.unchanged,
        }
    }
}
