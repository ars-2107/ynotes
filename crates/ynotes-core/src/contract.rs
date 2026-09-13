//! Shared versioned JSON payloads and mappings for CLI and MCP responses.
//!
//! Field changes require a [`CONTRACT_VERSION`] bump, contract snapshots, and
//! an update to `ynotes.schema.json`. Terminal rendering stays in the CLI.

use serde::Serialize;

use crate::anchor::{AnchorStatus, Rung, RungOutcome, RungResult};
use crate::confirm::ConfirmResult;
use crate::note::Scope;
use crate::query::ResolvedNote;
use crate::reanchor::ReanchorReport;
use crate::store::{IndexHealth, IndexStatus, MalformedNote};

/// The current `--json` contract version. Bumped when any field changes;
/// mirrored in `tests/agent_contract.rs` and `ynotes.schema.json`
/// (invariant #7).
pub const CONTRACT_VERSION: u8 = 1;

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
    /// The note's target path, repo-relative, forward-slashed.
    pub target: String,
    /// The JSON-facing scope word: `"range"` or `"file"` (see
    /// [`scope_word_json`]).
    pub scope: String,
    /// The resolver's verdict: `"anchored"`, `"drifted"`, or `"orphaned"`.
    pub status: &'static str,
    /// Agent convenience: `false` only for an `anchored` note, a pure
    /// function of `status`, since `drifted` and `orphaned` are both stale.
    pub stale: bool,
    /// `[start, end]` for a located region, `null` for an orphan.
    pub resolved_range: RangePair,
    /// Where the region was when the note was saved, present only when it
    /// moved (a `drifted` or `orphaned` note). Omitted for an `anchored` note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_range: Option<[u32; 2]>,
    /// Present only when this note surfaced under a file it was renamed *to*:
    /// the pre-rename path it is still stored under. Its presence tells an
    /// agent that `reanchor` would make the migration durable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relocated_from: Option<String>,
    /// The path the note resolved at. Emitted by every read that emits
    /// [`Self::relocated_from`], the two sides of a pending migration always
    /// travel together, so a consumer can act on it without knowing which
    /// call produced the note. On a file query it repeats the path that was
    /// asked about; on an inventory it is the only place the destination
    /// appears.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relocated_to: Option<String>,
    /// The note's context body, verbatim.
    pub body: String,
    /// Present only with `--explain`; omitted otherwise so the default schema
    /// is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rungs: Option<Vec<RungView>>,
}

/// The stable JSON object for a record under `notes/` that could not be read or
/// parsed. Shared by `query` and `list` so their `malformed[]` arrays
/// are byte-identical for the same input. `error` is free-form (for display),
/// a consumer branches on an entry *existing*, not on its text.
#[derive(Debug, Serialize)]
pub struct MalformedView {
    /// The unreadable record's full id, when the canonical record path carries
    /// one. This is the exact handle `ynotes delete` needs to purge it.
    pub id: Option<String>,
    /// The target path the index filed the record under. `null` when the record
    /// is also absent from the cache and its contents cannot be parsed.
    pub target: Option<String>,
    /// Store-relative path to the unreadable record. Always present, including
    /// when no safe id can be recovered from an irregular filename.
    pub path: String,
    /// Why the record could not be read or parsed, free-form, for display.
    pub error: String,
}

/// The status breakdown emitted by `--count` summary mode. A pure tally over a
/// resolved set, the cheap answer an agent reads to decide whether to pull
/// bodies, without paying the tokens for them. `total` equals
/// `anchored + drifted + orphaned`.
#[derive(Debug, Serialize)]
pub struct CountView {
    /// Total notes tallied, equals `anchored + drifted + orphaned`.
    pub total: u32,
    /// Notes whose code was found present and intact at the saved position.
    pub anchored: u32,
    /// Notes found, but moved and/or edited since they were written.
    pub drifted: u32,
    /// Notes no content rung could locate, surfaced anyway (invariant #4).
    pub orphaned: u32,
}

/// Drift between authoritative record files and the rebuildable by-path cache.
/// Both counts are zero for a healthy index. A read reports these values but
/// never repairs the cache.
#[derive(Debug, Serialize)]
pub struct IndexHealthView {
    /// Cache readability: `"healthy"`, `"missing"`, or `"malformed"`.
    pub status: &'static str,
    /// Record ids found on disk but absent from the index.
    pub recovered: usize,
    /// Index ids with no record file behind them.
    pub dangling: usize,
}

/// Map engine index health to the stable agent-facing shape.
#[must_use]
pub fn index_health_view(index: IndexHealth) -> IndexHealthView {
    IndexHealthView {
        status: match index.status {
            IndexStatus::Healthy => "healthy",
            IndexStatus::Missing => "missing",
            IndexStatus::Malformed => "malformed",
        },
        recovered: index.recovered,
        dangling: index.dangling,
    }
}

/// Maps an engine [`MalformedNote`] to its stable view.
#[must_use]
pub fn malformed_view(m: &MalformedNote) -> MalformedView {
    MalformedView {
        id: m.id.clone(),
        target: m.target.clone(),
        path: m.path.clone(),
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
        relocated_to: rn.relocated_to.clone(),
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
/// A single-line region has equal endpoints. Keep the stored [`Scope::Line`]
/// variant because changing it would invalidate existing content-addressed IDs.
#[must_use]
pub fn scope_word_json(scope: Scope) -> &'static str {
    match scope {
        Scope::Line | Scope::Range => "range",
        Scope::File => "file",
    }
}

/// First line of a note's body truncated at a stable 80 characters (with `…`).
///
/// Shapes the contract field `deleteData.body_excerpt`, so it lives beside the
/// payload types: the CLI (`delete`, `prune`) and the MCP `forget` tool all
/// render the field through this one function and cannot drift on the shape.
#[must_use]
pub fn body_excerpt(body: &str) -> String {
    const MAX: usize = 80;
    let first = body.lines().next().unwrap_or("");
    if first.chars().count() <= MAX {
        first.to_owned()
    } else {
        let truncated: String = first.chars().take(MAX).collect();
        format!("{truncated}…")
    }
}

/// The stable payload emitted after a note is explicitly confirmed against
/// current code.
#[derive(Debug, Serialize)]
pub struct ConfirmData {
    /// The confirmed note's current content-addressed id.
    pub id: String,
    /// The id superseded by this confirmation.
    pub previous_id: String,
    /// Store-relative target path.
    pub target: String,
    /// JSON-facing scope word.
    pub scope: String,
    /// Current reviewed region as `[start, end]`.
    pub range: [u32; 2],
    /// Whether the note's content-addressed identity changed.
    pub changed: bool,
    /// Whether the operation created a new record.
    pub created: bool,
}

/// Map an engine confirmation result to the shared CLI/MCP payload.
#[must_use]
pub fn confirm_view(result: &ConfirmResult) -> ConfirmData {
    ConfirmData {
        id: result.note.id.clone(),
        previous_id: result.previous_id.clone(),
        target: result.note.target.clone(),
        scope: scope_word_json(result.note.scope).to_owned(),
        range: [result.range.start(), result.range.end()],
        changed: result.changed,
        created: result.created,
    }
}

/// The `query --json` payload: the request echo plus every resolved note.
#[derive(Debug, Serialize)]
pub struct QueryData {
    /// Echo of the request (file and location) this payload answers.
    pub query: QuerySpecView,
    /// Every resolved note this query returns, in a single array, anchored,
    /// drifted, and orphaned together. Each entry carries `status` so a
    /// consumer that wants the historical matched/orphaned split groups by
    /// it (`status == "orphaned"` ⇒ was an orphan; everything else matched
    /// the queried interval under its scope). Unified in `v=5` so query and
    /// `list` share one shape, with orphans still always returned (the
    /// never-drop promise, invariant #4).
    pub notes: Vec<NoteView>,
    /// Records relevant to this file that could not be read or parsed. Skipped
    /// from `notes` (they cannot be resolved) but surfaced here
    /// so a corrupt record is never silently dropped from a query (invariant
    /// #4). Always present; empty when there is nothing to flag.
    pub malformed: Vec<MalformedView>,
    /// Index-cache drift observed while serving the query.
    pub index: IndexHealthView,
    /// Non-fatal advisories, populated, for example, when the target file
    /// does not exist or the index needs reindexing. Always present,
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
/// full-and-correct contract), only the output is condensed, a `count` object
/// and a `malformed` tally in place of the `notes`/`malformed` arrays.
#[derive(Debug, Serialize)]
pub struct QueryCountData {
    /// Echo of the request this summary answers.
    pub query: QuerySpecView,
    /// The status breakdown across the resolved set.
    pub count: CountView,
    /// How many relevant records could not be read, the count-mode
    /// analogue of the full payload's `malformed[]`, so an agent in summary mode
    /// still learns a corrupt record is present (invariant #4).
    pub malformed: usize,
    /// Index-cache drift observed while serving the query.
    pub index: IndexHealthView,
    /// Non-fatal advisories, empty when there is nothing to flag.
    pub warnings: Vec<String>,
}

/// The `list --json` payload: every note in the store with its anchor status.
#[derive(Debug, Serialize)]
pub struct ListData {
    /// Every resolved note, in inventory order.
    pub notes: Vec<NoteView>,
    /// Records under `notes/` that could not be read or parsed. Skipped
    /// from `notes` (they cannot be resolved) but surfaced here so a corrupt
    /// record is never silently omitted from an inventory (invariant #4).
    /// Always present; empty when there is nothing to flag.
    pub malformed: Vec<MalformedView>,
    /// Index-cache drift observed while serving the inventory.
    pub index: IndexHealthView,
    /// Non-fatal advisories, populated when a filter matched nothing or the
    /// index needs reindexing. Always present; empty when there is nothing to
    /// flag.
    pub warnings: Vec<String>,
}

/// `list --count --json` payload: the status breakdown across the listed set in
/// place of the `notes` array. Same resolution as [`ListData`], condensed.
#[derive(Debug, Serialize)]
pub struct ListCountData {
    /// The status breakdown across the listed set.
    pub count: CountView,
    /// How many records under `notes/` could not be read, the count-mode
    /// analogue of the full payload's `malformed[]` (invariant #4).
    pub malformed: usize,
    /// Index-cache drift observed while serving the inventory.
    pub index: IndexHealthView,
    /// Non-fatal advisories, empty when there is nothing to flag.
    pub warnings: Vec<String>,
}

/// The `show --json` payload: a single note resolved against current code.
#[derive(Debug, Serialize)]
pub struct ShowData {
    /// The resolved note.
    pub note: NoteView,
    /// Non-fatal advisories, populated, for example, when the note's target
    /// file does not exist on disk (the note then resolves `orphaned`). Always
    /// present; empty when there is nothing to flag.
    pub warnings: Vec<String>,
}

/// The `lookup --json` payload: the live note(s) matching a stable handle.
#[derive(Debug, Serialize)]
pub struct LookupData {
    /// The note(s) matching the `(target, body)` handle, each resolved against
    /// current code. A malformed record cannot be read to match either part, so
    /// it structurally cannot appear here, hence no `malformed` array.
    pub notes: Vec<NoteView>,
}

/// The `init --json` payload: where the selected store lives and whether this
/// invocation created it. Re-running init on an existing store also repairs its
/// managed merge guards, while `created` remains `false`.
#[derive(Debug, Serialize)]
pub struct InitData {
    /// Absolute path to the `.ynotes` store.
    pub root: String,
    /// Whether this invocation created the store.
    pub created: bool,
}

/// The `save --json` payload: a stable identity record for the saved note, for
/// an agent to capture and reference later. Serialised with `serde_json`, never
/// string interpolation: the `--json` surface is an agent contract and must stay
/// valid JSON for any target path (a quote in a filename must be escaped).
#[derive(Debug, Serialize)]
pub struct SaveData {
    /// The saved note's content-addressed id, a stable handle to reference it.
    pub id: String,
    /// The note's target path (repo-relative, forward-slashed).
    pub target: String,
    /// The JSON-facing scope word: `"range"` or `"file"`.
    pub scope: String,
    /// `[start, end]` the note was anchored to at save time.
    pub range: [u32; 2],
    /// `true` if this call created the note; `false` if an identical note
    /// already existed (`save` is idempotent, invariant #6).
    pub created: bool,
    /// `true` only when this call created the store at the Git root.
    pub store_created: bool,
    /// The other notes already on this file, earliest region first.
    ///
    /// Helps callers spot near-duplicates that identical-location superseding
    /// does not remove. Empty when the file has no other notes.
    pub neighbours: Vec<NeighbourView>,
}

/// One note already present on the file a save just wrote to.
#[derive(Debug, Serialize)]
pub struct NeighbourView {
    /// The existing note's id, pass it straight to `update` or `delete`.
    pub id: String,
    /// `[start, end]` the existing note is anchored to.
    pub range: [u32; 2],
    /// The first line of its body, truncated, enough to recognise a
    /// restatement without pulling every body on the file into the payload.
    pub body_excerpt: String,
}

impl NeighbourView {
    /// Project the store's live notes into the contract's neighbour shape.
    #[must_use]
    pub fn project(notes: &[crate::note::Note]) -> Vec<Self> {
        notes
            .iter()
            .map(|n| Self {
                id: n.id.clone(),
                range: [
                    n.bundle.position.range.start(),
                    n.bundle.position.range.end(),
                ],
                body_excerpt: body_excerpt(&n.body),
            })
            .collect()
    }
}

/// The `files --json` payload: which files carry notes, including verified renames.
///
/// Omits ranges and statuses because inventory does not resolve every note.
/// Use query or list for current locations.
#[derive(Debug, Serialize)]
pub struct FilesData {
    /// Annotated files, most-annotated first (ties broken by path).
    pub files: Vec<AnnotatedFileView>,
    /// Total notes across every file.
    pub total: usize,
    /// How many records under `notes/` could not be read or parsed.
    pub malformed: usize,
    /// Cache condition, reported but never repaired by a read.
    pub index: IndexHealthView,
}

/// One annotated file in the [`FilesData`] inventory.
#[derive(Debug, Serialize)]
pub struct AnnotatedFileView {
    /// The target path, store-relative and forward-slashed.
    pub target: String,
    /// How many notes are discoverable at it, including pending renames.
    pub notes: usize,
}

impl FilesData {
    /// Project the engine's inventory into the contract shape.
    #[must_use]
    pub fn new(inventory: &crate::query::FileInventory) -> Self {
        Self {
            files: inventory
                .files
                .iter()
                .map(|f| AnnotatedFileView {
                    target: f.target.clone(),
                    notes: f.notes,
                })
                .collect(),
            total: inventory.total,
            malformed: inventory.malformed,
            index: index_health_view(inventory.index),
        }
    }
}

/// Stable JSON payload for `delete --json`.
///
/// `requested` is the count of ids the caller passed; the four category arrays
/// partition the outcome (each requested id appears in exactly one). Always
/// emitted in full, empty arrays included, so a consumer never has to branch
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
    /// Ids of corrupt records purged by exact id, a success, not a failure, so
    /// it does not flip the exit code.
    pub deleted_unreadable: Vec<String>,
    /// Requested prefixes that matched more than one note, a failure.
    pub ambiguous: Vec<AmbiguousView>,
    /// Requested ids that matched no note, a failure.
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
/// for it, named rather than silently dropped.
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
    /// Notes whose selectors needed nothing but whose bodies are still
    /// unreviewed. Populated on every pass, unlike `review_required` inside
    /// `changed`/`relocated`, which a note carries only on the pass that
    /// refreshed it.
    pub review_pending: Vec<PendingReviewView>,
    /// Notes already current (resolved to where they were captured).
    pub unchanged: usize,
}

/// A note the pass left alone because its location was already right, but whose
/// body has not been confirmed against the code it points at. Its `id` is the
/// live one, nothing was rewritten, so it can be passed straight to
/// `confirm`.
#[derive(Debug, Serialize)]
pub struct PendingReviewView {
    /// The note's target path.
    pub target: String,
    /// The note's id, directly confirmable.
    pub id: String,
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
    /// Whether current code differs from the note's explicit review basis.
    pub review_required: bool,
}

/// JSON projection of a note moved to a renamed file. Its own partition (not
/// folded into `changed`) because a relocation changes the note's `target`, a
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
    /// Whether current code differs from the note's explicit review basis.
    pub review_required: bool,
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
                    review_required: c.review_required,
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
                    review_required: r.review_required,
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
            review_pending: r
                .review_pending
                .iter()
                .map(|p| PendingReviewView {
                    target: p.target.clone(),
                    id: p.id.clone(),
                })
                .collect(),
            unchanged: r.unchanged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_excerpt_takes_the_first_line() {
        assert_eq!(body_excerpt("first\nsecond"), "first");
    }

    #[test]
    fn body_excerpt_truncates_at_80_chars_with_ellipsis() {
        let long = "x".repeat(100);
        let e = body_excerpt(&long);
        assert_eq!(e.chars().count(), 81);
        assert!(e.ends_with('…'));
    }
}
