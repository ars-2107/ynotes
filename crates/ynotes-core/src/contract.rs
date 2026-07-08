//! The wire shape of the versioned `--json` agent contract.
//!
//! Every `--json` payload a front-end emits is one of the structs defined
//! here, wrapped in the success/failure envelope. The contract is a stable,
//! versioned agent surface (invariant #7): a field change bumps
//! [`CONTRACT_VERSION`] and, in the same change, updates both the
//! `tests/agent_contract.rs` snapshot and the published `ynotes.schema.json`.
//!
//! These shapes live in the engine, not in any one front-end, because every
//! front-end must serialise *the same bytes*: the CLI's `--json` output and the
//! MCP server's tool results are the same contract. Defining the payloads once,
//! here, is what makes that guarantee structural rather than a promise two
//! code paths have to keep in step.
//!
//! Only the JSON shapes and the mapping functions that build them live here.
//! Turning a resolved note into a coloured terminal line is presentation and
//! stays in each front-end — the engine has no opinion on how a face renders
//! text.

use serde::Serialize;

use crate::anchor::{AnchorStatus, Rung, RungOutcome, RungResult};
use crate::note::Scope;
use crate::query::ResolvedNote;
use crate::reanchor::ReanchorReport;
use crate::store::MalformedNote;

/// The current `--json` contract version. Bumped when any field changes;
/// mirrored in `tests/agent_contract.rs` and `ynotes.schema.json`
/// (invariant #7).
pub const CONTRACT_VERSION: u8 = 11;

/// `[a, b]` for a resolved range, `null` for an orphan.
type RangePair = Option<[u32; 2]>;

/// One rung's outcome, for `--explain`.
#[derive(Debug, Serialize)]
pub struct RungView {
    /// Which rung produced this outcome (`"git"`, `"quote"`, `"structural"`,
    /// `"fuzzy"`, or `"position"`).
    pub rung: &'static str,
    /// What the rung concluded: `"hit"`, `"miss"`, or `"skipped"`.
    pub result: &'static str,
    /// The range the rung located, present only on a hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: RangePair,
    /// The rung's self-assessed confidence in its hit, present only on a hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<u8>,
}

/// The stable per-note JSON object (and the data the text renderer formats).
#[derive(Debug, Serialize)]
pub struct NoteView {
    /// The note's content-addressed id.
    pub id: String,
    /// The note's target path — repo-relative, forward-slashed.
    pub target: String,
    /// The JSON-facing scope word: `"range"` or `"file"` (see
    /// [`scope_word_json`]).
    pub scope: String,
    /// The resolver's verdict: `"anchored"`, `"drifted"`, or `"orphaned"`.
    pub status: &'static str,
    /// Agent convenience: `false` only for an `anchored` note — a pure
    /// function of `status`, since `drifted` and `orphaned` are both stale.
    pub stale: bool,
    /// `[start, end]` for a located region, `null` for an orphan.
    pub resolved_range: RangePair,
    /// Where the region was when the note was saved, present only when it
    /// moved (a `drifted` or `orphaned` note). Omitted for an `anchored` note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_range: Option<[u32; 2]>,
    /// Present only when this note surfaced under a file it was renamed *to*:
    /// the pre-rename path it is still stored under. Omitted otherwise, so the
    /// default note shape is unchanged. Its presence tells an agent the note
    /// migrated here and a `reanchor` would make that durable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relocated_from: Option<String>,
    /// The note's context body, verbatim.
    pub body: String,
    /// Present only with `--explain`; omitted otherwise so the default schema
    /// is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rungs: Option<Vec<RungView>>,
}

/// The stable JSON object for a record the index pointed at that could not be
/// read or parsed. Shared by `query` and `list` so their `malformed[]` arrays
/// are byte-identical for the same input. `error` is free-form (for display) —
/// a consumer branches on an entry *existing*, not on its text.
#[derive(Debug, Serialize)]
pub struct MalformedView {
    /// The unreadable record's id — the *full* id, the exact handle
    /// `ynotes delete` needs to purge the record.
    pub id: String,
    /// The target path the index filed the record under.
    pub target: String,
    /// Why the record could not be read or parsed — free-form, for display.
    pub error: String,
}

/// The status breakdown emitted by `--count` summary mode. A pure tally over a
/// resolved set — the cheap answer an agent reads to decide whether to pull
/// bodies, without paying the tokens for them. `total` equals
/// `anchored + drifted + orphaned`.
#[derive(Debug, Serialize)]
pub struct CountView {
    /// Total notes tallied — equals `anchored + drifted + orphaned`.
    pub total: u32,
    /// Notes whose code was found present and intact at the saved position.
    pub anchored: u32,
    /// Notes found, but moved and/or edited since they were written.
    pub drifted: u32,
    /// Notes no content rung could locate, surfaced anyway (invariant #4).
    pub orphaned: u32,
}

/// Maps an engine [`MalformedNote`] to its stable view.
#[must_use]
pub fn malformed_view(m: &MalformedNote) -> MalformedView {
    MalformedView {
        id: m.id.clone(),
        target: m.target.clone(),
        error: m.error.clone(),
    }
}

/// Maps a resolved note to its view; includes the rung vector if `explain`.
#[must_use]
pub fn note_view(rn: &ResolvedNote, explain: bool) -> NoteView {
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

/// Maps one engine rung outcome to its `--explain` view.
fn rung_view(o: &RungOutcome) -> RungView {
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

/// Tally resolved notes by anchor status for `--count`. Shared by `query` and
/// `list` so the two summarise identically.
#[must_use]
pub fn count_view<'a>(notes: impl Iterator<Item = &'a ResolvedNote>) -> CountView {
    let mut c = CountView {
        total: 0,
        anchored: 0,
        drifted: 0,
        orphaned: 0,
    };
    for rn in notes {
        c.total += 1;
        match rn.resolution.status {
            AnchorStatus::Anchored => c.anchored += 1,
            AnchorStatus::Drifted { .. } => c.drifted += 1,
            AnchorStatus::Orphaned { .. } => c.orphaned += 1,
        }
    }
    c
}

/// JSON-facing scope: collapses `Line` and `Range` to a single `"range"` so
/// the agent contract carries one representation for a `[start, end]` region.
/// A consumer that cares about user intent can recover it from `range[0] ==
/// range[1]`; before v3 we exposed two variants for the same shape, forcing
/// every caller to branch. The internal [`Scope::Line`] enum stays — note
/// ids are content-hashed over `(target, scope, bundle, body)` (invariant
/// #6), so changing the enum would invalidate every existing single-line
/// note's id; this is presentation-only.
#[must_use]
pub fn scope_word_json(scope: Scope) -> &'static str {
    match scope {
        Scope::Line | Scope::Range => "range",
        Scope::File => "file",
    }
}

/// The `query --json` payload: the request echo plus every resolved note.
#[derive(Debug, Serialize)]
pub struct QueryData {
    /// Echo of the request (file and location) this payload answers.
    pub query: QuerySpecView,
    /// Every resolved note this query returns, in a single array — anchored,
    /// drifted, and orphaned together. Each entry carries `status` so a
    /// consumer that wants the historical matched/orphaned split groups by
    /// it (`status == "orphaned"` ⇒ was an orphan; everything else matched
    /// the queried interval under its scope). Unified in `v=5` so query and
    /// `list` share one shape, with orphans still always returned (the
    /// never-drop promise — invariant #4).
    pub notes: Vec<NoteView>,
    /// Records for this file the index pointed at that could not be read or
    /// parsed. Skipped from `notes` (they cannot be resolved) but surfaced here
    /// so a corrupt record is never silently dropped from a query (invariant
    /// #4). Always present; empty when there is nothing to flag.
    pub malformed: Vec<MalformedView>,
    /// Non-fatal advisories — populated, for example, when the target file
    /// does not exist (almost always a typo, but `exit=0` is the right Unix
    /// signal for "no match" so the warning lives in-band). Always present,
    /// empty when there is nothing to flag, so consumers do not have to
    /// branch on key presence.
    pub warnings: Vec<String>,
}

/// The queried location, echoed back so a consumer can pair the payload with
/// the request it answered.
#[derive(Debug, Serialize)]
pub struct QuerySpecView {
    /// The queried file, as the caller spelled it.
    pub file: String,
    /// The location argument (`""` for a whole-file query).
    pub at: String,
}

/// `query --count --json` payload: the status breakdown instead of the note
/// bodies. Same resolution as [`QueryData`] (rename-following included, per the
/// full-and-correct contract), only the output is condensed — a `count` object
/// and a `malformed` tally in place of the `notes`/`malformed` arrays.
#[derive(Debug, Serialize)]
pub struct QueryCountData {
    /// Echo of the request this summary answers.
    pub query: QuerySpecView,
    /// The status breakdown across the resolved set.
    pub count: CountView,
    /// How many records the index pointed at could not be read — the count-mode
    /// analogue of the full payload's `malformed[]`, so an agent in summary mode
    /// still learns a corrupt record is present (invariant #4).
    pub malformed: usize,
    /// Non-fatal advisories, empty when there is nothing to flag.
    pub warnings: Vec<String>,
}

/// The `list --json` payload: every note in the store with its anchor status.
#[derive(Debug, Serialize)]
pub struct ListData {
    /// Every resolved note, in inventory order.
    pub notes: Vec<NoteView>,
    /// Records the index pointed at that could not be read or parsed. Skipped
    /// from `notes` (they cannot be resolved) but surfaced here so a corrupt
    /// record is never silently omitted from an inventory (invariant #4).
    /// Always present; empty when there is nothing to flag.
    pub malformed: Vec<MalformedView>,
    /// Non-fatal advisories, populated when a filter matched nothing (the
    /// human form already distinguishes these cases; the array gives a
    /// machine consumer the same signal). Always present; empty when there is
    /// nothing to flag.
    pub warnings: Vec<String>,
}

/// `list --count --json` payload: the status breakdown across the listed set in
/// place of the `notes` array. Same resolution as [`ListData`], condensed.
#[derive(Debug, Serialize)]
pub struct ListCountData {
    /// The status breakdown across the listed set.
    pub count: CountView,
    /// How many records the index pointed at could not be read — the count-mode
    /// analogue of the full payload's `malformed[]` (invariant #4).
    pub malformed: usize,
    /// Non-fatal advisories, empty when there is nothing to flag.
    pub warnings: Vec<String>,
}

/// The `show --json` payload: a single note resolved against current code.
#[derive(Debug, Serialize)]
pub struct ShowData {
    /// The resolved note.
    pub note: NoteView,
    /// Non-fatal advisories — populated, for example, when the note's target
    /// file does not exist on disk (the note then resolves `orphaned`). Always
    /// present; empty when there is nothing to flag.
    pub warnings: Vec<String>,
}

/// The `lookup --json` payload: the live note(s) matching a stable handle.
#[derive(Debug, Serialize)]
pub struct LookupData {
    /// The note(s) matching the `(target, body)` handle, each resolved against
    /// current code. A malformed record cannot be read to match either part, so
    /// it structurally cannot appear here — hence no `malformed` array.
    pub notes: Vec<NoteView>,
}

/// The `save --json` payload: a stable identity record for the saved note, for
/// an agent to capture and reference later. Serialised with `serde_json`, never
/// string interpolation: the `--json` surface is an agent contract and must stay
/// valid JSON for any target path (a quote in a filename must be escaped).
#[derive(Debug, Serialize)]
pub struct SaveData {
    /// The saved note's content-addressed id — a stable handle to reference it.
    pub id: String,
    /// The note's target path (repo-relative, forward-slashed).
    pub target: String,
    /// The JSON-facing scope word: `"range"` or `"file"`.
    pub scope: String,
    /// `[start, end]` the note was anchored to at save time.
    pub range: [u32; 2],
    /// `true` if this call created the note; `false` if an identical note
    /// already existed (`save` is idempotent — invariant #6).
    pub created: bool,
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
#[derive(Debug, Serialize)]
pub struct DeleteData {
    /// How many ids the caller passed; the category arrays below partition them.
    pub requested: usize,
    /// Notes removed, each with a preview.
    pub deleted: Vec<DeletedView>,
    /// Ids of corrupt records purged by exact id — a success, not a failure, so
    /// it does not flip the exit code.
    pub deleted_unreadable: Vec<String>,
    /// Requested prefixes that matched more than one note — a failure.
    pub ambiguous: Vec<AmbiguousView>,
    /// Requested ids that matched no note — a failure.
    pub not_found: Vec<String>,
    /// Whether this was a preview (nothing written).
    pub dry_run: bool,
}

/// A note that was (or, in a dry run, would be) removed.
#[derive(Debug, Serialize)]
pub struct DeletedView {
    /// The id or prefix the caller passed to select this note.
    pub requested: String,
    /// The full id of the note that was removed.
    pub id: String,
    /// The removed note's target path.
    pub target: String,
    /// The JSON-facing scope word: `"range"` or `"file"`.
    pub scope: &'static str,
    /// `[start, end]` the note was anchored to.
    pub range: [u32; 2],
    /// The first line of the note's body, truncated for a preview.
    pub body_excerpt: String,
}

/// A requested prefix that matched more than one note, so nothing was deleted
/// for it — named rather than silently dropped.
#[derive(Debug, Serialize)]
pub struct AmbiguousView {
    /// The ambiguous prefix the caller passed.
    pub requested: String,
    /// The full ids the prefix matched.
    pub candidates: Vec<String>,
}

/// JSON projection of a [`ReanchorReport`]. Defined here (not by deriving
/// `Serialize` on the engine type) so the agent contract stays a deliberate
/// surface a refactor inside the engine cannot accidentally widen.
#[derive(Debug, Serialize)]
pub struct ReanchorView {
    /// Whether this was a preview (nothing written).
    pub dry_run: bool,
    /// Notes refreshed in place (or that would be).
    pub changed: Vec<ChangedView>,
    /// Notes moved to a renamed file (or that would be).
    pub relocated: Vec<RelocatedView>,
    /// Notes deliberately left untouched, with why.
    pub skipped: Vec<SkippedView>,
    /// Notes already current (resolved to where they were captured).
    pub unchanged: usize,
}

/// A note refreshed in place: its region moved or its text was edited, so every
/// selector was re-captured against the current code.
#[derive(Debug, Serialize)]
pub struct ChangedView {
    /// The note's target path.
    pub target: String,
    /// Id before re-anchoring.
    pub old_id: String,
    /// Id after (content-addressed over the refreshed bundle).
    pub new_id: String,
    /// Where the note used to be anchored.
    pub from: [u32; 2],
    /// Where it is now.
    pub to: [u32; 2],
}

/// JSON projection of a note moved to a renamed file. Its own partition (not
/// folded into `changed`) because a relocation changes the note's `target` — a
/// distinct event a consumer branches on separately.
#[derive(Debug, Serialize)]
pub struct RelocatedView {
    /// The target path the note was filed under before the rename.
    pub from_target: String,
    /// The target path it now lives at (the file's current name).
    pub to_target: String,
    /// Id before relocating.
    pub old_id: String,
    /// Id after (content-addressed over the new target and refreshed bundle).
    pub new_id: String,
    /// Where the region sat in the old file at save time.
    pub from: [u32; 2],
    /// Where it resolved to in the renamed file.
    pub to: [u32; 2],
}

/// A note deliberately left untouched by `reanchor`, with why.
#[derive(Debug, Serialize)]
pub struct SkippedView {
    /// The note's target path.
    pub target: String,
    /// The skipped note's id.
    pub id: String,
    /// A stable, lower-kebab-case classification an agent can branch on
    /// without parsing prose; the human form lives in [`Self::reason`].
    pub reason_code: &'static str,
    /// The free-form, user-facing explanation.
    pub reason: &'static str,
}

impl From<&ReanchorReport> for ReanchorView {
    fn from(r: &ReanchorReport) -> Self {
        Self {
            dry_run: r.dry_run,
            changed: r
                .changed
                .iter()
                .map(|c| ChangedView {
                    target: c.target.clone(),
                    old_id: c.old_id.clone(),
                    new_id: c.new_id.clone(),
                    from: [c.from.start(), c.from.end()],
                    to: [c.to.start(), c.to.end()],
                })
                .collect(),
            relocated: r
                .relocated
                .iter()
                .map(|r| RelocatedView {
                    from_target: r.from_target.clone(),
                    to_target: r.to_target.clone(),
                    old_id: r.old_id.clone(),
                    new_id: r.new_id.clone(),
                    from: [r.from.start(), r.from.end()],
                    to: [r.to.start(), r.to.end()],
                })
                .collect(),
            skipped: r
                .skipped
                .iter()
                .map(|s| SkippedView {
                    target: s.target.clone(),
                    id: s.id.clone(),
                    reason_code: s.reason.code(),
                    reason: s.reason.human(),
                })
                .collect(),
            unchanged: r.unchanged,
        }
    }
}
