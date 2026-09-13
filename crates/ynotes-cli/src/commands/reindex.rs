//! `ynotes reindex`, rebuild the by-path index from the notes on disk.
//!
//! All policy lives in the engine ([`ynotes::Store::reindex`]); this only
//! renders the report and chooses the exit-neutral wording for a dry run. The
//! index is a cache: this is the repair pass that reconciles it back to the
//! note records, which are the source of truth.

use std::io::Write as _;

use serde::Serialize;
use ynotes::{ReindexReport, Store};

use super::json_envelope;
use super::render::short_id;
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Run (or, with `dry_run`, preview) the reindex pass.
///
/// # Errors
///
/// [`CommandError::Engine`] if the store cannot be read or the rebuilt index
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
    let report = store.reindex(dry_run).map_err(CommandError::Engine)?;

    let mut out = std::io::stdout().lock();
    let (recovered_verb, dropped_verb) = if report.dry_run {
        ("would recover", "would drop pointer")
    } else {
        ("recovered", "dropped pointer")
    };

    for id in &report.recovered {
        writeln!(
            out,
            "{} {}",
            paint(recovered_verb, Colour::Green),
            short_id(id),
        )?;
    }
    for id in &report.dangling {
        writeln!(
            out,
            "{} {}",
            paint(dropped_verb, Colour::Yellow),
            short_id(id),
        )?;
    }
    for path in &report.malformed {
        // An observation, not an action, phrased the same in a dry run, since
        // the file is unreadable either way and was never indexed.
        writeln!(
            out,
            "{} {}",
            paint("unreadable", Colour::Red),
            path.display(),
        )?;
    }
    writeln!(
        out,
        "{}",
        paint(
            &format!(
                "{} scanned, {} indexed, {} recovered, {} stale dropped, {} unreadable{}",
                report.scanned,
                report.indexed,
                report.recovered.len(),
                report.dangling.len(),
                report.malformed.len(),
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
    let report = store.reindex(dry_run).map_err(CommandError::Engine)?;
    json_envelope::print_success(&ReindexView::from(&report))
}

/// JSON projection of [`ReindexReport`]. Defined locally (not by deriving
/// `Serialize` on the engine type) so the agent contract stays a deliberate
/// surface a refactor inside the engine cannot accidentally widen.
#[derive(Serialize)]
struct ReindexView<'a> {
    dry_run: bool,
    scanned: usize,
    indexed: usize,
    recovered: &'a [String],
    dangling: &'a [String],
    malformed: Vec<String>,
}

impl<'a> From<&'a ReindexReport> for ReindexView<'a> {
    fn from(r: &'a ReindexReport) -> Self {
        Self {
            dry_run: r.dry_run,
            scanned: r.scanned,
            indexed: r.indexed,
            recovered: &r.recovered,
            dangling: &r.dangling,
            // Lossy is safe here: these paths live under `.ynotes/notes/` and
            // are reported for a human to inspect, not re-opened by key.
            malformed: r
                .malformed
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        }
    }
}
