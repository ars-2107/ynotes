//! Shared *text* rendering for `query` and `list`: one note → a coloured line.
//!
//! Presentation only (binary side). The engine hands back a structured
//! [`ResolvedNote`]; this turns it into a coloured, human-readable line. The
//! JSON contract those commands also emit lives in [`ynotes::contract`], so
//! both faces serialise the same bytes; this module is purely the terminal
//! form.

use std::io::Write;

use ynotes::contract::{CountView, note_view};
use ynotes::{AnchorStatus, MalformedNote, ResolvedNote};

use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Writes one malformed-record advisory in the human text format. Flagged in
/// red and pointing at the remedy, so a corrupt record is loud, not hidden
/// (invariant #4). The body is unreadable, so there is nothing to show but the
/// id, the target, and why, and the *full* id (not the short form), since it
/// is the exact handle `ynotes delete` needs to purge the record.
pub(crate) fn write_text_malformed(
    out: &mut impl Write,
    m: &MalformedNote,
) -> Result<(), CommandError> {
    let id = m.id.as_deref().unwrap_or("<id unknown>");
    let target = m.target.as_deref().unwrap_or("<target unknown>");
    writeln!(
        out,
        "{id} {target}  [{}]",
        paint("unreadable ✗", Colour::Red),
    )?;
    writeln!(out, "  {}", paint(&m.path, Colour::Dim))?;
    writeln!(out, "  {}", paint(&m.error, Colour::Dim))?;
    if let Some(id) = &m.id {
        writeln!(
            out,
            "  {}",
            paint(&format!("remove with: ynotes delete {id}"), Colour::Dim,),
        )?;
    } else {
        // No id could be recovered, so `delete` has no handle to take, it
        // requires the full 64-character id. Without this line the reader is
        // told their store is broken and given nothing to do about it, which is
        // the one thing a diagnostic must not do.
        writeln!(
            out,
            "  {}",
            paint(
                "no id could be recovered, so `ynotes delete` cannot address \
                 this record; remove the file above by hand, then run \
                 `ynotes reindex`",
                Colour::Dim,
            ),
        )?;
    }
    writeln!(out)?;
    Ok(())
}

/// Writes one note in the human text format, coloured by status.
pub(crate) fn write_text_note(
    out: &mut impl Write,
    rn: &ResolvedNote,
    explain: bool,
) -> Result<(), CommandError> {
    let v = note_view(rn, explain);
    let range = v
        .resolved_range
        .map_or_else(|| "?".to_owned(), |[a, b]| format!("{a}:{b}"));
    let (tag, colour) = match rn.resolution.status {
        AnchorStatus::Anchored => {
            // Intact but moved: still green, the move is information not alarm.
            let tag = match v.previous_range {
                Some([a, b]) => format!("anchored was {a}:{b}"),
                None => "anchored".to_owned(),
            };
            (tag, Colour::Green)
        }
        AnchorStatus::Drifted { from } => {
            // "was X" only when the region actually moved. A content-only
            // drift, an in-place edit, or any note just refreshed by
            // `reanchor`, resolves to the same range it was saved at, where
            // "was 8:11" beside a resolved "8:11" is noise, not information.
            let tag = if rn.resolution.range == Some(from) {
                "drifted ⚠".to_owned()
            } else {
                format!("drifted ⚠ was {}:{}", from.start(), from.end())
            };
            (tag, Colour::Yellow)
        }
        AnchorStatus::Orphaned { last_known } => (
            format!(
                "orphaned ✗ last seen {}:{}",
                last_known.start(),
                last_known.end()
            ),
            Colour::Red,
        ),
    };
    // Scope prefix on the range so a file-scoped note and a range note that
    // happens to span the same lines are visually distinct (`save` and
    // `delete` already say `file:` / `range:`; without it the two scopes are
    // indistinguishable in `list` / `query` text and only recoverable from
    // `--json`'s `scope` field).
    writeln!(
        out,
        "{} {}:{}  [{}]",
        v.target,
        v.scope,
        range,
        paint(&tag, colour),
    )?;
    if let Some(from) = &v.relocated_from {
        // This note is stored under `from` but surfaced here because the
        // queried file was renamed from it. Name that explicitly, and point at
        // the verb that makes the migration durable.
        writeln!(
            out,
            "  {}",
            paint(
                &format!(
                    "relocated from {from}{} (run `ynotes reanchor` to migrate)",
                    v.relocated_to
                        .as_ref()
                        .map_or_else(String::new, |to| format!(" to {to}")),
                ),
                Colour::Cyan,
            ),
        )?;
    }
    for line in v.body.lines() {
        writeln!(out, "  {line}")?;
    }
    if let Some(rungs) = &v.rungs {
        writeln!(out, "  {}", paint("rungs:", Colour::Dim))?;
        for r in rungs {
            let detail = match (r.range, r.score) {
                (Some([a, b]), Some(s)) => format!("{} {a}:{b} ({s})", r.result),
                _ => r.result.to_owned(),
            };
            writeln!(
                out,
                "    {}",
                paint(&format!("{}: {detail}", r.rung), Colour::Dim)
            )?;
        }
    }
    writeln!(out)?;
    Ok(())
}

/// Render a `--count` summary as human text: one tally line, plus a red
/// unreadable-records line when records under `notes/` could not be read, so
/// corruption is flagged in the summary too, never hidden
/// (invariant #4). Shared by `query` and `list`.
pub(crate) fn write_count_line(
    out: &mut impl Write,
    count: &CountView,
    malformed: usize,
) -> Result<(), CommandError> {
    let noun = if count.total == 1 { "note" } else { "notes" };
    writeln!(
        out,
        "{} {noun}: {} anchored, {} drifted, {} orphaned",
        count.total, count.anchored, count.drifted, count.orphaned,
    )?;
    if malformed > 0 {
        let mnoun = if malformed == 1 { "record" } else { "records" };
        writeln!(
            out,
            "{}",
            paint(&format!("{malformed} unreadable {mnoun}"), Colour::Red),
        )?;
    }
    Ok(())
}

/// Hex-id short form for the human text: the first 12 characters, enough to
/// disambiguate in any realistic store while staying scannable. Shared
/// between `delete`, `update`, `prune`, and `reanchor` so the four
/// commands' rendered ids cannot drift.
pub(crate) fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}
