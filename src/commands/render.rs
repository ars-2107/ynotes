//! Shared rendering for `query` and `list`: one note → text or JSON.
//!
//! Presentation only (binary side). The engine hands back a structured
//! [`ResolvedNote`]; turning it into a coloured line or a stable JSON object
//! lives here so both commands render identically and the JSON contract has a
//! single definition.

use std::io::Write;

use serde::Serialize;
use ynotes::{AnchorStatus, MalformedNote, ResolvedNote, Rung, RungResult, Scope};

use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// `[a, b]` for a resolved range, `null` for an orphan.
type RangePair = Option<[u32; 2]>;

/// One rung's outcome, for `--explain`.
#[derive(Serialize)]
pub(crate) struct RungView {
    rung: &'static str,
    result: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: RangePair,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<u8>,
}

/// The stable per-note JSON object (and the data the text renderer formats).
#[derive(Serialize)]
pub(crate) struct NoteView {
    pub(crate) id: String,
    pub(crate) target: String,
    pub(crate) scope: String,
    pub(crate) status: &'static str,
    /// Agent convenience: `false` only for an `anchored` note — a pure
    /// function of `status`, since `drifted` and `orphaned` are both stale.
    pub(crate) stale: bool,
    pub(crate) resolved_range: RangePair,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) previous_range: Option<[u32; 2]>,
    /// Present only when this note surfaced under a file it was renamed *to*:
    /// the pre-rename path it is still stored under. Omitted otherwise, so the
    /// default note shape is unchanged. Its presence tells an agent the note
    /// migrated here and a `reanchor` would make that durable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) relocated_from: Option<String>,
    pub(crate) body: String,
    /// Present only with `--explain`; omitted otherwise so the default schema
    /// is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rungs: Option<Vec<RungView>>,
}

/// The stable JSON object for a record the index pointed at that could not be
/// read or parsed. Shared by `query` and `list` so their `malformed[]` arrays
/// are byte-identical for the same input. `error` is free-form (for display) —
/// a consumer branches on an entry *existing*, not on its text.
#[derive(Serialize)]
pub(crate) struct MalformedView {
    pub(crate) id: String,
    pub(crate) target: String,
    pub(crate) error: String,
}

/// Maps an engine [`MalformedNote`] to its stable view.
pub(crate) fn malformed_view(m: &MalformedNote) -> MalformedView {
    MalformedView {
        id: m.id.clone(),
        target: m.target.clone(),
        error: m.error.clone(),
    }
}

/// Writes one malformed-record advisory in the human text format. Flagged in
/// red and pointing at the remedy, so a corrupt record is loud, not hidden
/// (invariant #4). The body is unreadable, so there is nothing to show but the
/// id, the target, and why — and the *full* id (not the short form), since it
/// is the exact handle `ynotes delete` needs to purge the record.
pub(crate) fn write_text_malformed(
    out: &mut impl Write,
    m: &MalformedNote,
) -> Result<(), CommandError> {
    writeln!(
        out,
        "{} {}  [{}]",
        m.id,
        m.target,
        paint("unreadable ✗", Colour::Red),
    )?;
    writeln!(out, "  {}", paint(&m.error, Colour::Dim))?;
    writeln!(
        out,
        "  {}",
        paint(&format!("remove with: ynotes delete {}", m.id), Colour::Dim,),
    )?;
    writeln!(out)?;
    Ok(())
}

/// Maps a resolved note to its view; includes the rung vector if `explain`.
pub(crate) fn note_view(rn: &ResolvedNote, explain: bool) -> NoteView {
    let (status, previous) = match rn.resolution.status {
        AnchorStatus::Anchored => ("anchored", None),
        AnchorStatus::Drifted { from } => ("drifted", Some([from.start(), from.end()])),
        AnchorStatus::Orphaned { last_known } => {
            ("orphaned", Some([last_known.start(), last_known.end()]))
        }
    };
    let stale = !matches!(rn.resolution.status, AnchorStatus::Anchored);
    NoteView {
        id: rn.note.id.clone(),
        target: rn.note.target.clone(),
        scope: scope_word_json(rn.note.scope).to_owned(),
        status,
        stale,
        resolved_range: rn.resolution.range.map(|r| [r.start(), r.end()]),
        previous_range: previous,
        relocated_from: rn.relocated_from.clone(),
        body: rn.note.body.clone(),
        rungs: explain.then(|| rn.resolution.rungs.iter().map(rung_view).collect()),
    }
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
        AnchorStatus::Anchored => ("anchored".to_owned(), Colour::Green),
        AnchorStatus::Drifted { from } => {
            // "was X" only when the region actually moved. A content-only
            // drift — an in-place edit, or any note just refreshed by
            // `reanchor` — resolves to the same range it was saved at, where
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
                &format!("relocated from {from} (run `ynotes reanchor` to migrate)"),
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

fn rung_view(o: &ynotes::RungOutcome) -> RungView {
    let rung = match o.rung {
        Rung::Git => "git",
        Rung::Structural => "structural",
        Rung::Quote => "quote",
        Rung::Fuzzy => "fuzzy",
        Rung::Position => "position",
    };
    match o.result {
        RungResult::Hit { range, score } => RungView {
            rung,
            result: "hit",
            range: Some([range.start(), range.end()]),
            score: Some(score),
        },
        RungResult::Miss => RungView {
            rung,
            result: "miss",
            range: None,
            score: None,
        },
        RungResult::Skipped => RungView {
            rung,
            result: "skipped",
            range: None,
            score: None,
        },
    }
}

/// Hex-id short form for the human text: the first 12 characters — enough to
/// disambiguate in any realistic store while staying scannable. Shared
/// between `delete`, `update`, `prune`, and `reanchor` so the four
/// commands' rendered ids cannot drift.
pub(crate) fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// First line of a note's body truncated at a stable 80 chars (with `…`).
/// Shared between `delete` and `prune` so their `body_excerpt` field is the
/// same string for the same input — agents that diff the two outputs need a
/// stable shape.
pub(crate) fn body_excerpt(body: &str) -> String {
    const MAX: usize = 80;
    let first = body.lines().next().unwrap_or("");
    if first.chars().count() <= MAX {
        first.to_owned()
    } else {
        let truncated: String = first.chars().take(MAX).collect();
        format!("{truncated}…")
    }
}

/// JSON-facing scope: collapses `Line` and `Range` to a single `"range"` so
/// the agent contract carries one representation for a `[start, end]` region.
/// A consumer that cares about user intent can recover it from `range[0] ==
/// range[1]`; before v3 we exposed two variants for the same shape, forcing
/// every caller to branch. The internal [`Scope::Line`] enum stays — note
/// ids are content-hashed over `(target, scope, bundle, body)` (invariant
/// #6), so changing the enum would invalidate every existing single-line
/// note's id; this is presentation-only.
pub(crate) fn scope_word_json(scope: Scope) -> &'static str {
    match scope {
        Scope::Line | Scope::Range => "range",
        Scope::File => "file",
    }
}
