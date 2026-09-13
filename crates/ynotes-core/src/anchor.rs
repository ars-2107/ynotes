//! Resolve a saved [`SelectorBundle`] against current source text.
//!
//! Content matching establishes whether the region survives. Git transport
//! can corroborate a match, but cannot decide alone because a deleted range
//! may transport onto unrelated code at the deletion point.
//!
//! The saved-position rung is diagnostic only. It derives from the stored
//! range, so treating it as independent evidence would be circular.
//!
//! The result is [`Anchored`](AnchorStatus::Anchored) when located and intact,
//! [`Drifted`](AnchorStatus::Drifted) when located but moved or changed, or
//! [`Orphaned`](AnchorStatus::Orphaned) when identity cannot be established.
//! Per-rung scores are diagnostics, not a combined confidence score.

use crate::git::{GitContext, Transport};
use crate::selector::SelectorBundle;
use crate::source::{LineRange, SourceFile};
use crate::structural;

/// Which rung produced an outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// R1, transported through git history.
    Git,
    /// R2, located by its exact text (+ surrounding context).
    Quote,
    /// R4, located by its position in the syntax tree.
    Structural,
    /// R3, located approximately after its contents were edited.
    Fuzzy,
    /// R5, the saved position. Surfaced in `--explain` so an agent can see
    /// what the user originally pointed at alongside the other rungs'
    /// verdicts; never consulted by `reconcile`, because it is derived from
    /// the saved range (clamped when the file shrank), so treating its
    /// overlap with a content rung as corroboration would be circular.
    Position,
}

/// What a rung found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RungResult {
    /// Located, with a 0..=100 self-assessed score.
    Hit {
        /// Where this rung believes the region now is.
        range: LineRange,
        /// The rung's own confidence in that range.
        score: u8,
    },
    /// The rung ran but did not find the region.
    Miss,
    /// The rung could not run (e.g. not a git repo); contributes nothing and
    /// is not held against the result.
    Skipped,
}

/// One rung's contribution, kept for `--explain` and auditing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RungOutcome {
    /// The rung.
    pub rung: Rung,
    /// Its result.
    pub result: RungResult,
}

/// The resolved status of a note's anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorStatus {
    /// A content rung found the region's code present and intact, at the
    /// position the note was saved at.
    Anchored,
    /// The region was found but it moved and/or its contents changed since
    /// the note was written.
    Drifted {
        /// Where the region was when the note was saved.
        from: LineRange,
    },
    /// No content rung could locate the region. The note is surfaced anyway,
    /// with its last known position, never silently discarded.
    Orphaned {
        /// The position recorded when the note was saved.
        last_known: LineRange,
    },
}

/// The resolver's verdict for one note.
///
/// Deliberately carries no numeric confidence: the [`status`](Resolution::status)
/// describes the code anchor, not whether the note's claims are true. Even
/// intact code can depend on changed callers or configuration elsewhere.
#[derive(Debug, Clone)]
pub struct Resolution {
    /// The resolved range, or `None` when [`AnchorStatus::Orphaned`].
    pub range: Option<LineRange>,
    /// Anchored / drifted / orphaned, the verdict.
    pub status: AnchorStatus,
    /// Every rung's outcome, in rung order, for `--explain`.
    pub rungs: Vec<RungOutcome>,
}

impl Resolution {
    /// Whether a rung that establishes the region's *identity*, rather than its
    /// resemblance, located it.
    ///
    /// Git transport and the structural path both *name* the region; an exact
    /// quote proves its text is byte-intact. Fuzzy and an ambiguous quote (55)
    /// assert only similarity and can select a same-shaped sibling.
    /// [`Rung::Position`] never corroborates anything.
    ///
    /// Trust-advancing writes gate on this. A review basis captured against a
    /// merely similar region records that a body was checked against code
    /// nobody actually read, and the previous basis is gone, so the mistake is
    /// unrecoverable. Reads are unaffected: a similarity hit is still a
    /// perfectly good `drifted` answer, because `drifted` already tells the
    /// reader to re-read the lines.
    /// The score bars are the same ones the resolver uses to accept a rung as
    /// a *witness*, deliberately: a signal too weak to corroborate a location
    /// is too weak to advance trust in a body. Accepting any git or structural
    /// hit would let a score-70 transport through, the "carried a replaced
    /// range onto its change point" case, which is evidence the region was
    /// *edited*, not that it is still there, and a sub-79 structural hit,
    /// which is a same-named construct in a different scope.
    #[must_use]
    pub fn is_identified(&self) -> bool {
        self.rungs
            .iter()
            .any(|outcome| match (outcome.rung, outcome.result) {
                (Rung::Git, RungResult::Hit { score, .. }) => score >= GIT_WITNESS_MIN,
                (Rung::Structural, RungResult::Hit { score, .. }) => {
                    score >= STRUCTURAL_WITNESS_MIN
                }
                (Rung::Quote, RungResult::Hit { score, .. }) => score >= QUOTE_EXACT,
                _ => false,
            })
    }
}

/// Quote-rung score at or above which the hit is an *exact*, confidently
/// disambiguated match, 95 (a unique occurrence) or 90 (context-confirmed
/// among several), rather than an ambiguous best guess (55). Only an exact
/// quote hit proves the region's text is byte-intact.
const QUOTE_EXACT: u8 = 90;

/// Why an exact quote candidate is or is not safe to treat as the saved
/// region. Kept categorical so reconciliation policy does not emerge from an
/// unexplained numeric threshold; the score is only its diagnostic rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuoteEvidence {
    /// Independently identified exact text remains at the saved range.
    SavedPosition,
    /// Saved prefix/suffix uniquely identify this occurrence.
    ContextDisambiguated,
    /// Exact text was unique at capture and remains unique now.
    UniqueAcrossVersions,
    /// The text matches, but its identity is not independently established.
    WeakCandidate,
}

impl QuoteEvidence {
    const fn score(self) -> u8 {
        match self {
            Self::SavedPosition => 95,
            Self::ContextDisambiguated | Self::UniqueAcrossVersions => 90,
            Self::WeakCandidate => 55,
        }
    }
}

/// Structural-rung score meaning the enclosing construct's content
/// fingerprint, node kinds *and* identifier/literal text, is identical: the
/// code is intact bar whitespace and comments. `structural_score` reaches
/// this only for a similarity of 1.0.
const STRUCTURAL_INTACT: u8 = 95;

/// Git-transport score at or above which the rung followed the region's *own
/// surviving lines* to their new home, rather than merely echoing a
/// deletion/modification point. An `Unchanged` transport (100) or a `Moved`
/// one whose endpoint lines were untouched (90) tracked real content; a
/// `touched` transport (70) carried a *replaced* range onto its change point,
/// git's own low-confidence signal, and is no evidence the region is still
/// there. Only a git witness at or above this bar may corroborate a moved
/// fuzzy match in [`reconcile`] step 2. Below it, git echoes a line number
/// without proving content, exactly the "carries a deleted range onto its
/// deletion point" case the module doctrine says must never decide a verdict.
const GIT_WITNESS_MIN: u8 = 90;

/// Structural-rung score at or above which the hit re-located the *same
/// construct in its original scope*, its ancestor chain matched exactly,
/// rather than a same-named construct in a *different* scope (a sibling
/// `Bar::build` when the note was on `Foo::build`). `structural_score` gives an
/// exact-chain match at least this (0.80 → 79 when the body was edited, 1.0 →
/// 95 when intact) and a different-chain namesake at most 71, so this bar
/// separates the two cleanly. Only an exact-chain hit is independent evidence
/// the *noted* construct is at fuzzy's location; a lower structural hit merely
/// found a namesake elsewhere and must not witness a moved fuzzy match in
/// [`reconcile`] step 2, the structural twin of the touched-git case.
const STRUCTURAL_WITNESS_MIN: u8 = 79;

/// The false-positive bar a resolution runs under.
///
/// Same-file resolution and rename corroboration ask different questions, so
/// they trust the fuzzy rung's "did not move" signal differently, see
/// [`reconcile`] step 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolveMode {
    /// Same-file resolution: a fuzzy hit at the saved line range is trusted as
    /// "did not move" without an independent witness.
    Normal,
    /// Corroboration against a *renamed* destination: the saved line range
    /// carries no meaning in a different file, so the "did not move" shortcut is
    /// withheld and a moved fuzzy hit still needs an independent witness
    /// (invariant #10).
    Relocation,
}

/// Resolves where `bundle`'s region is in `file`, optionally using `git` for
/// the R1 transport rung.
///
/// Never fails: an unresolvable note becomes [`AnchorStatus::Orphaned`], a
/// successful `Resolution`, because turning "lost" into an `Err` would invite
/// a caller to drop it.
#[must_use]
pub fn resolve(bundle: &SelectorBundle, file: &SourceFile, git: Option<&GitContext>) -> Resolution {
    resolve_with(bundle, file, git, ResolveMode::Normal)
}

/// Resolves a note against a renamed destination, using only content evidence
/// that is strong enough to migrate across a path boundary.
///
/// Rename following has a stricter false-positive bar than ordinary same-file
/// resolution: a fuzzy hit at the saved line range is acceptable for an
/// in-place edit, but not for proving the old file's note belongs to a new
/// path. A same-line replacement after a rename can look similar enough to
/// fuzzy while being unrelated code, so callers use this mode before surfacing
/// or persisting a relocation.
pub(crate) fn resolve_for_relocation(bundle: &SelectorBundle, file: &SourceFile) -> Resolution {
    resolve_with(bundle, file, None, ResolveMode::Relocation)
}

/// Shared implementation behind [`resolve`] and [`resolve_for_relocation`]:
/// runs each rung once and hands the outcomes to [`reconcile`] under `mode`.
fn resolve_with(
    bundle: &SelectorBundle,
    file: &SourceFile,
    git: Option<&GitContext>,
    mode: ResolveMode,
) -> Resolution {
    let saved = bundle.position.range;
    let rungs = vec![
        RungOutcome {
            rung: Rung::Git,
            result: rung_git(bundle, git),
        },
        RungOutcome {
            rung: Rung::Quote,
            result: rung_quote(bundle, file, git),
        },
        RungOutcome {
            rung: Rung::Structural,
            result: rung_structural(bundle, file),
        },
        RungOutcome {
            rung: Rung::Fuzzy,
            result: rung_fuzzy(bundle, file),
        },
        RungOutcome {
            rung: Rung::Position,
            result: rung_position(bundle, file),
        },
    ];

    reconcile(saved, rungs, mode)
}

/// R1: ask git to transport the region's baseline range to the working file.
fn rung_git(bundle: &SelectorBundle, git: Option<&GitContext>) -> RungResult {
    let (Some(sel), Some(ctx)) = (bundle.git.as_ref(), git) else {
        return RungResult::Skipped;
    };
    // Transport the git selector's *own* range, captured in its baseline
    // commit's coordinates. `position.range` is the working-tree range, which
    // `reanchor` rewrites; pairing it with an unchanged baseline commit is
    // what desynced the rung. A note from before the field falls back to it.
    let baseline = sel.range.unwrap_or(bundle.position.range);
    match ctx.transport(&sel.commit, &sel.path, baseline) {
        Transport::Unchanged(range) => RungResult::Hit { range, score: 100 },
        Transport::Moved { to, touched } => RungResult::Hit {
            range: to,
            score: if touched { 70 } else { 90 },
        },
        Transport::Unknown => RungResult::Miss,
    }
}

/// R4: ask the syntax tree where the region's enclosing construct went.
fn rung_structural(bundle: &SelectorBundle, file: &SourceFile) -> RungResult {
    let Some(sel) = bundle.structural.as_ref() else {
        return RungResult::Skipped;
    };
    match structural::locate(sel, &file.text_all(), sel.language) {
        Some((range, sim)) => RungResult::Hit {
            range,
            score: structural_score(sim),
        },
        None => RungResult::Miss,
    }
}

/// Maps a structural similarity in `[0.5, 1.0]` to a self-score in
/// `[55, 95]`. Only an identical fingerprint (1.0) reaches `STRUCTURAL_INTACT`;
/// below that the structural rung is a location hint, not content proof.
fn structural_score(sim: f64) -> u8 {
    let t = ((sim - 0.5) / 0.5).clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (55.0 + t * 40.0).round() as u8
    }
}

/// R2: find the region's exact text, disambiguated by its surrounding lines.
fn rung_quote(bundle: &SelectorBundle, file: &SourceFile, git: Option<&GitContext>) -> RungResult {
    let needle: Vec<&str> = bundle.quote.exact.split('\n').collect();
    if file.line_count() == 0 {
        return RungResult::Miss;
    }
    let hay = file.lines_slice();
    if needle.len() > hay.len() {
        return RungResult::Miss;
    }

    let mut matches: Vec<usize> = Vec::new();
    for start in 0..=(hay.len() - needle.len()) {
        if hay[start..start + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(a, b)| a == b)
        {
            matches.push(start);
        }
    }

    match matches.as_slice() {
        [] => RungResult::Miss,
        [only] => {
            let range = to_range(*only, needle.len());
            let has_context = !bundle.quote.prefix.is_empty() || !bundle.quote.suffix.is_empty();
            // Current uniqueness alone is insufficient: a duplicated line can
            // disappear from its original function and leave one identical
            // sibling behind. Capture-time uniqueness distinguishes that case
            // from a genuinely unique block whose neighbours changed during a
            // reorder. Schema-v1 notes have no such evidence and retain the
            // conservative context/witness requirement.
            let unique_at_capture = bundle.quote.occurrences_at_capture == Some(1)
                || (bundle.quote.occurrences_at_capture.is_none()
                    && bundle.git.as_ref().is_some_and(|saved| {
                        git.and_then(|ctx| {
                            ctx.exact_occurrences_at(
                                &saved.commit,
                                &saved.path,
                                &bundle.quote.exact,
                            )
                        }) == Some(1)
                    }));
            let context_identifies = has_context && context_fits(bundle, hay, *only, needle.len());
            let evidence =
                if range == bundle.position.range && (context_identifies || unique_at_capture) {
                    QuoteEvidence::SavedPosition
                } else if context_identifies {
                    QuoteEvidence::ContextDisambiguated
                } else if unique_at_capture {
                    QuoteEvidence::UniqueAcrossVersions
                } else {
                    QuoteEvidence::WeakCandidate
                };
            RungResult::Hit {
                range,
                score: evidence.score(),
            }
        }
        many => {
            // Ambiguous: prefer the occurrence whose surrounding lines match
            // the saved prefix/suffix; fall back to the one nearest the
            // original position.
            let context_matched: Vec<usize> = many
                .iter()
                .copied()
                .filter(|&s| context_fits(bundle, hay, s, needle.len()))
                .collect();
            let (chosen, score) = match context_matched.as_slice() {
                [one] => (*one, 90),
                _ => (nearest(many, bundle.position.range.start()), 55),
            };
            RungResult::Hit {
                range: to_range(chosen, needle.len()),
                score,
            }
        }
    }
}

/// R5: the saved position. Computed for `--explain` so an agent can see what
/// the user originally pointed at, alongside what the other rungs concluded.
/// `reconcile` never reads this rung: it is derived from the saved range
/// (clamped when the file shrank), so treating its overlap with any content
/// rung as corroboration would be circular. Kept here, not removed, because
/// the cost is negligible and the agreement vector is more useful when it
/// surfaces every rung.
fn rung_position(bundle: &SelectorBundle, file: &SourceFile) -> RungResult {
    let r = bundle.position.range;
    if r.end() <= file.line_count() {
        RungResult::Hit {
            range: r,
            score: 20,
        }
    } else if file.line_count() >= 1 {
        // File got shorter: clamp so it still nudges tie-breaks, weakly.
        match LineRange::new(r.start().min(file.line_count()), file.line_count()) {
            Ok(clamped) => RungResult::Hit {
                range: clamped,
                score: 10,
            },
            Err(_) => RungResult::Miss,
        }
    } else {
        RungResult::Miss
    }
}

/// Largest saved region (in lines) R3 will try to relocate. Above this the
/// region is effectively whole-file: fuzzy adds nothing the other rungs do
/// not, and the cost is not worth paying.
const FUZZY_MAX_LINES: usize = 400;

/// A saved line is only a useful anchor if it is distinctive (long enough,
/// not pure punctuation) and not ubiquitous in the file.
const ANCHOR_MIN_LEN: usize = 3;
/// Saved lines occurring more than this many times in the file are too common
/// to anchor on.
const ANCHOR_MAX_OCCUR: usize = 30;
/// At most this many anchor candidates are tried (rarest, longest first).
const ANCHOR_CANDIDATES: usize = 6;

/// Minimum line-LCS similarity to accept a multi-line fuzzy match. Biased
/// conservative: a borderline match should orphan honestly (invariant #4),
/// not silently point at the wrong block.
const FUZZY_ACCEPT_MULTI: f64 = 0.55;
/// Stricter threshold for a single-line region (one line is little evidence).
const FUZZY_ACCEPT_SINGLE: f64 = 0.60;
/// How far from the saved line a single-line fuzzy search looks.
const SINGLE_LINE_WINDOW: usize = 60;

/// Character similarity at or above which two lines count as *the same line,
/// edited* rather than two different lines. Set so that renaming an identifier
/// or two within a line still leaves it recognisable, while an unrelated line
/// scores below it. Used by the LCS to make candidate scoring rename-tolerant.
const LINE_SIM: f64 = 0.6;
/// Half-width of the position-anchored fallback window in the multi-line path:
/// the saved block is also scored aligned at every file offset within
/// `±POS_WINDOW` of its saved start, so a region with no exact-surviving line
/// (every identifier renamed) still yields a candidate.
const POS_WINDOW: usize = 40;
/// Largest saved region (in lines) for which the position-window similarity
/// search runs. Above this a region reliably keeps exact-surviving distinctive
/// lines, so the exact-anchor candidates suffice and the matrix is not built.
const SIM_MAX_LINES: usize = 60;

/// Whether two lines are similar enough to count as the same line after an
/// edit, character similarity at or above [`LINE_SIM`].
fn line_match(a: &str, b: &str) -> bool {
    ratio(a, b) >= LINE_SIM
}

/// R3: locate a region whose *contents* were edited (R2's exact match fails
/// the instant a character inside the region changes).
///
/// Method, anchor on the
/// rarest surviving line (common lines carry no localising information),
/// extrapolate the region window from that anchor, and score it by a
/// line-level longest-common-subsequence similarity over trimmed lines. Lines
/// are matched by [`line_match`] (character similarity), not exact equality,
/// so an in-place rename of every local, which changes many lines at once and
/// can leave no exact-surviving anchor, is still recovered rather than lost.
///
/// Two candidate sources feed the score and the best across both wins: the
/// exact-anchor hypotheses above, and, for a region of at most
/// [`SIM_MAX_LINES`] lines, the saved block aligned at every offset within
/// [`POS_WINDOW`] of its saved position, so a wholly-renamed region with no
/// exact survivor still produces a candidate.
///
/// The self-score is capped below exact/git so a fuzzy-only resolution is
/// always `drifted`, never `anchored`, the contents really did change.
fn rung_fuzzy(bundle: &SelectorBundle, file: &SourceFile) -> RungResult {
    let saved: Vec<String> = bundle
        .quote
        .exact
        .split('\n')
        .map(|l| l.trim_end().to_owned())
        .collect();
    let hay: Vec<String> = file
        .lines_slice()
        .iter()
        .map(|l| l.trim_end().to_owned())
        .collect();

    if hay.is_empty() {
        return RungResult::Miss;
    }
    if saved.len() > FUZZY_MAX_LINES {
        return RungResult::Skipped;
    }

    if let Some(hit) = fuzzy_hit_at_saved_range(&saved, &hay, bundle.position.range) {
        return hit;
    }
    if saved.len() == 1 {
        return fuzzy_single_line(&saved[0], &hay, bundle.position.range);
    }

    // Occurrence count of each saved line within the file.
    let mut occ: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in &hay {
        *occ.entry(line.as_str()).or_insert(0) += 1;
    }

    // Distinctive saved lines, rarest then longest first.
    let mut anchors: Vec<(usize, usize, usize)> = saved
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            let c = *occ.get(line.as_str())?;
            (line.len() >= ANCHOR_MIN_LEN && (1..=ANCHOR_MAX_OCCUR).contains(&c)).then_some((
                c,
                line.len(),
                i,
            ))
        })
        .collect();
    anchors.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    anchors.truncate(ANCHOR_CANDIDATES);
    // No surviving distinctive line is not, on its own, proof the region is
    // gone: a uniform rename of every local changes every line at once. The
    // exact-anchor pass below simply contributes nothing, and the
    // position-window pass is left to recover the region, or to confirm,
    // honestly, that nothing similar survives.

    let mut best: Option<(f64, LineRange)> = None;
    let mut consider = |start0: usize, lcs: usize, lo: usize, hi: usize| {
        #[allow(clippy::cast_precision_loss)]
        let sim = lcs as f64 / saved.len() as f64;
        if best.as_ref().is_none_or(|(b, _)| sim > *b)
            && let Ok(r) = LineRange::new(
                u32::try_from(start0 + lo + 1).unwrap_or(1),
                u32::try_from(start0 + hi + 1).unwrap_or(1),
            )
        {
            best = Some((sim, r));
        }
    };

    // Candidate source 1, exact-anchor hypotheses: align the saved block so a
    // distinctive saved line sits on its identical file line.
    for &(_, _, i) in &anchors {
        for (j, hline) in hay.iter().enumerate() {
            if *hline != saved[i] {
                continue;
            }
            // Hypothesis: saved line `i` is now file line `j`.
            let start0 = j.saturating_sub(i);
            let end0 = (start0 + saved.len()).min(hay.len());
            if start0 >= end0 {
                continue;
            }
            let window = &hay[start0..end0];
            let (lcs, lo, hi) = lcs_with_extent(&saved, window);
            consider(start0, lcs, lo, hi);
        }
    }

    // Candidate source 2, position-anchored fallback: a region whose every
    // identifier was renamed may have no exact-surviving line, so the anchor
    // loop yields nothing. Score the saved block aligned at every offset within
    // `POS_WINDOW` of its saved start. A boolean match matrix, built once,
    // bounds `ratio` calls to roughly `saved.len() * (2*POS_WINDOW + saved.len())`.
    if saved.len() <= SIM_MAX_LINES {
        let saved_start0 = (bundle.position.range.start().saturating_sub(1)) as usize;
        let base = saved_start0.saturating_sub(POS_WINDOW);
        // The matrix spans every hay line any in-range offset's window can
        // reach: from `base` to the furthest offset's end.
        let last_offset = (saved_start0 + POS_WINDOW).min(hay.len().saturating_sub(1));
        let span_end = (last_offset + saved.len()).min(hay.len());
        if base < span_end {
            let span = span_end - base;
            let matrix: Vec<Vec<bool>> = saved
                .iter()
                .map(|s| (0..span).map(|j| line_match(s, &hay[base + j])).collect())
                .collect();
            for offset in base..=last_offset {
                let end0 = (offset + saved.len()).min(hay.len());
                if offset >= end0 {
                    continue;
                }
                let (lcs, lo, hi) = matrix_lcs_with_extent(&matrix, offset - base, end0 - offset);
                consider(offset, lcs, lo, hi);
            }
        }
    }

    match best {
        Some((sim, range)) if sim >= FUZZY_ACCEPT_MULTI => RungResult::Hit {
            range,
            score: fuzzy_score(sim, FUZZY_ACCEPT_MULTI),
        },
        _ => RungResult::Miss,
    }
}

/// Returns the fuzzy result directly when the saved text still occupies its
/// original range. The full candidate search can only rediscover this same
/// maximum-score hit, while its LCS cost grows sharply for whole-file notes.
fn fuzzy_hit_at_saved_range(
    saved: &[String],
    hay: &[String],
    range: LineRange,
) -> Option<RungResult> {
    let start = (range.start().saturating_sub(1)) as usize;
    let end = range.end() as usize;
    (end <= hay.len() && saved == &hay[start..end]).then_some(RungResult::Hit {
        range,
        score: fuzzy_score(1.0, FUZZY_ACCEPT_MULTI),
    })
}

/// Single-line region: there is no sequence to align, so fall back to a
/// character-similarity scan in a window around the saved line.
fn fuzzy_single_line(saved: &str, hay: &[String], pos: LineRange) -> RungResult {
    let centre = (pos.start().saturating_sub(1)) as usize;
    let lo = centre.saturating_sub(SINGLE_LINE_WINDOW);
    let hi = (centre + SINGLE_LINE_WINDOW + 1).min(hay.len());
    let mut best: Option<(f64, usize)> = None;
    for (idx, line) in hay.iter().enumerate().take(hi).skip(lo) {
        let s = ratio(saved, line);
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, idx));
        }
    }
    match best {
        Some((s, idx)) if s >= FUZZY_ACCEPT_SINGLE => {
            let l = u32::try_from(idx + 1).unwrap_or(1);
            LineRange::new(l, l).map_or(RungResult::Miss, |range| RungResult::Hit {
                range,
                score: fuzzy_score(s, FUZZY_ACCEPT_SINGLE),
            })
        }
        _ => RungResult::Miss,
    }
}

/// Maps a similarity in `[accept, 1.0]` to a self-score in `[45, 80]`. Capped
/// below 90 so fuzzy never reaches the "anchored" fidelity bar.
fn fuzzy_score(sim: f64, accept: f64) -> u8 {
    let t = ((sim - accept) / (1.0 - accept)).clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (45.0 + t * 35.0).round() as u8
    }
}

/// Longest common subsequence length over an abstract grid plus the first and
/// last column indices that participate in it (the matched extent, so a
/// region that grew or shrank is reported at its true new span).
///
/// `cell(si, wi)` answers whether saved row `si` matches window column `wi`,
/// abstracted so both the on-demand path ([`lcs_with_extent`]) and the
/// precomputed-matrix path ([`matrix_lcs_with_extent`]) share the DP + backtrack
/// body. The match predicate is whatever the caller plugs in; nothing in this
/// function assumes exact equality.
fn lcs_dp(sn: usize, wn: usize, cell: impl Fn(usize, usize) -> bool) -> (usize, usize, usize) {
    let mut dp = vec![vec![0u16; wn + 1]; sn + 1];
    for si in (0..sn).rev() {
        for wi in (0..wn).rev() {
            dp[si][wi] = if cell(si, wi) {
                dp[si + 1][wi + 1] + 1
            } else {
                dp[si + 1][wi].max(dp[si][wi + 1])
            };
        }
    }
    // Backtrack to find which columns of the window participate in the LCS.
    let (mut si, mut wi) = (0usize, 0usize);
    let (mut lo, mut hi, mut seen) = (wn.saturating_sub(1), 0usize, false);
    while si < sn && wi < wn {
        if cell(si, wi) {
            lo = lo.min(wi);
            hi = hi.max(wi);
            seen = true;
            si += 1;
            wi += 1;
        } else if dp[si + 1][wi] >= dp[si][wi + 1] {
            si += 1;
        } else {
            wi += 1;
        }
    }
    let lcs = usize::from(dp[0][0]);
    if seen {
        (lcs, lo, hi)
    } else {
        (lcs, 0, wn.saturating_sub(1))
    }
}

/// LCS over two line slices, lines compared with [`line_match`] (character
/// similarity) so a heavily-but-uniformly edited region, every identifier
/// renamed, still scores as present rather than gone.
fn lcs_with_extent(saved: &[String], window: &[String]) -> (usize, usize, usize) {
    lcs_dp(saved.len(), window.len(), |si, wi| {
        line_match(&saved[si], &window[wi])
    })
}

/// [`lcs_with_extent`] driven by a precomputed boolean match matrix.
///
/// `matrix[i][j]` is `line_match(saved[i], hay[base + j])` for the whole
/// searched hay span; `col0` is the matrix column the window starts at and
/// `wn` its line count. The position-window loop calls this once per offset, so
/// the costly [`ratio`] runs over the span exactly once rather than per offset.
///
/// # Panics
///
/// Panics if `col0 + wn` exceeds a matrix row's length, callers derive both
/// from the same span and never overrun it.
fn matrix_lcs_with_extent(matrix: &[Vec<bool>], col0: usize, wn: usize) -> (usize, usize, usize) {
    lcs_dp(matrix.len(), wn, |si, wi| matrix[si][col0 + wi])
}

/// Normalised character-similarity of two strings, `1.0 - lev/maxlen`, in
/// `[0, 1]`. Underlies both the single-line fuzzy scan and [`line_match`], so
/// the multi-line LCS tolerates an edited line; always over bounded windows.
fn ratio(lhs: &str, rhs: &str) -> f64 {
    let lc: Vec<char> = lhs.chars().collect();
    let rc: Vec<char> = rhs.chars().collect();
    let (ln, rn) = (lc.len(), rc.len());
    if ln == 0 && rn == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=rn).collect();
    let mut cur = vec![0usize; rn + 1];
    for li in 1..=ln {
        cur[0] = li;
        for ri in 1..=rn {
            let cost = usize::from(lc[li - 1] != rc[ri - 1]);
            cur[ri] = (prev[ri] + 1).min(cur[ri - 1] + 1).min(prev[ri - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let lev = prev[rn];
    #[allow(clippy::cast_precision_loss)]
    {
        1.0 - lev as f64 / ln.max(rn) as f64
    }
}

/// Builds a 1-based inclusive range from a 0-based start index and length.
/// Lengths come from real file slices, so this cannot overflow in practice.
fn to_range(start0: usize, len: usize) -> LineRange {
    let start = u32::try_from(start0 + 1).unwrap_or(1);
    let end = u32::try_from(start0 + len).unwrap_or(start);
    LineRange::new(start, end.max(start))
        .unwrap_or_else(|_| LineRange::new(start, start).expect("start >= 1 by construction"))
}

/// Whether the saved prefix/suffix lines bracket the candidate at `start0`.
fn context_fits(bundle: &SelectorBundle, hay: &[String], start0: usize, len: usize) -> bool {
    let pre = &bundle.quote.prefix;
    let suf = &bundle.quote.suffix;
    let before_ok = pre.is_empty() || {
        let pl = pre.split('\n').count();
        start0 >= pl && hay[start0 - pl..start0].join("\n") == *pre
    };
    let after_ok = suf.is_empty() || {
        let sl = suf.split('\n').count();
        let end = start0 + len;
        end + sl <= hay.len() && hay[end..end + sl].join("\n") == *suf
    };
    before_ok && after_ok
}

/// The candidate (0-based start) nearest the original 1-based line.
fn nearest(starts: &[usize], original_line: u32) -> usize {
    let target = original_line.saturating_sub(1) as usize;
    starts
        .iter()
        .copied()
        .min_by_key(|&s| s.abs_diff(target))
        .unwrap_or(0)
}

/// The hit `(range, score)` for rung `want`, or `None` if it missed, was
/// skipped, or is absent. A free function, not a closure, so the borrow of
/// `rungs` is released before `reconcile` moves it into the [`Resolution`].
fn rung_hit(rungs: &[RungOutcome], want: Rung) -> Option<(LineRange, u8)> {
    rungs.iter().find_map(|o| match o.result {
        RungResult::Hit { range, score } if o.rung == want => Some((range, score)),
        _ => None,
    })
}

/// Combines the rung outcomes into one verdict.
///
/// A decision procedure, not a weighted sum. Three steps:
///
/// 1. **No content evidence ⇒ orphaned.** No content rung hit (no `quote`,
///    no `fuzzy`) and no intact `structural` fingerprint means the region's
///    code is not demonstrably present. `git` transports a deleted range
///    onto its deletion point and `position` can clamp into a now-shorter
///    file, so they may echo a line number without proving anything about
///    content. Surfaced with its last known position, never dropped.
/// 2. **Fuzzy moved without an independent witness ⇒ orphaned.** Fuzzy's
///    LCS will happily land on a same-shaped sibling, a second
///    `function f(user) { … return signJwt(…); }`, so a fuzzy-only hit
///    that *moved* from the saved range demands corroboration from a rung
///    whose evidence is independent of fuzzy itself and re-located the *noted*
///    region, not a look-alike: an *untouched* `git` transport (score
///    `>= GIT_WITNESS_MIN`, one that followed surviving lines, not a
///    `touched` transport that merely carried a replaced range onto its
///    deletion point), or an *exact-chain* `structural` hit (score
///    `>= STRUCTURAL_WITNESS_MIN`, the same construct in its original scope,
///    not a same-named sibling elsewhere), with a range overlapping fuzzy's.
///    In normal same-file resolution, fuzzy hitting the *exact* saved range
///    needs no such witness if no ambiguous exact quote survives: it means
///    the position-window scoring picked the
///    saved offset over every other alignment, which is the rung's own evidence
///    of "did not move." Relocation mode disables that exception because a
///    renamed file can replace the old region with similar, unrelated code at
///    the same lines.
///    `position`, by contrast, *is* the saved range; treating its overlap
///    with fuzzy as corroboration is circular, so it is excluded.
/// 3. **Located ⇒ anchored or drifted.** The range is taken from the most
///    authoritative content rung, exact `quote`, else `fuzzy`, else
///    `structural`. An exact quote at `QUOTE_EXACT`, or an identical
///    structural fingerprint, at the saved position is `anchored`; anything
///    else located is `drifted`.
fn reconcile(saved: LineRange, rungs: Vec<RungOutcome>, mode: ResolveMode) -> Resolution {
    let raw_quote = rung_hit(&rungs, Rung::Quote);
    let fuzzy = rung_hit(&rungs, Rung::Fuzzy);
    let structural = rung_hit(&rungs, Rung::Structural);
    let git = rung_hit(&rungs, Rung::Git);

    // A low-scoring quote is an exact string match but not a securely
    // identified region. This happens for duplicate text and for a unique
    // occurrence that lost the saved context. Accept it only when an
    // independent rung located the same region. A surviving sibling can
    // inherit both the text and the saved line number after a deletion.
    let quote = raw_quote.filter(|(range, score)| {
        *score >= QUOTE_EXACT
            || git.is_some_and(|(r, s)| s >= GIT_WITNESS_MIN && r.overlaps(*range))
            || structural.is_some_and(|(r, s)| s >= STRUCTURAL_WITNESS_MIN && r.overlaps(*range))
    });

    // A structural hit at full score means the enclosing construct's content
    // fingerprint, node kinds *and* identifier/literal text, is identical:
    // the code is intact bar whitespace and comments. Below that, structural
    // has only recognised a construct of the same name and shape, which is no
    // proof the noted region itself survived, a partial deletion leaves the
    // containing function in place.
    let structural_intact = matches!(structural, Some((_, s)) if s >= STRUCTURAL_INTACT);

    // Step 1, no content evidence at all.
    if !(quote.is_some() || fuzzy.is_some() || structural_intact) {
        return Resolution {
            range: None,
            status: AnchorStatus::Orphaned { last_known: saved },
            rungs,
        };
    }

    // Step 2, fuzzy as the sole evidence. Trust it iff it stayed at the
    // saved range exactly, or another rung independently agrees on its
    // location. See the procedure comment above for why position is not a
    // witness here.
    if quote.is_none()
        && !structural_intact
        && let Some((fr, _)) = fuzzy
    {
        // Fuzzy must not rescue an exact match whose identity the quote
        // rung could not establish, even at the original line number.
        let stayed_put = mode == ResolveMode::Normal && fr == saved && raw_quote.is_none();
        // A witness counts only if it re-located the *noted* region, not a
        // look-alike. A git hit qualifies when it *followed surviving
        // content* (score >= GIT_WITNESS_MIN); a `touched` transport (70)
        // merely carried a replaced range onto its change point, the
        // deletion-point echo the module doctrine forbids from deciding a
        // verdict. A structural hit qualifies when its ancestor chain
        // matched exactly (score >= STRUCTURAL_WITNESS_MIN); a lower,
        // different-chain hit only found a same-named construct in another
        // scope. Either low-confidence hit could otherwise rescue a fuzzy
        // match that landed on a coincidental same-shaped sibling.
        let independent = git.is_some_and(|(r, s)| s >= GIT_WITNESS_MIN && r.overlaps(fr))
            || structural.is_some_and(|(r, s)| s >= STRUCTURAL_WITNESS_MIN && r.overlaps(fr));
        if !(stayed_put || independent) {
            return Resolution {
                range: None,
                status: AnchorStatus::Orphaned { last_known: saved },
                rungs,
            };
        }
    }

    // Located: the most authoritative content rung wins the range. Exact text
    // beats an approximate match beats a structural recognition.
    let range = quote.or(fuzzy).or(structural).map_or(saved, |(r, _)| r);

    // `anchored` = located, intact, and unmoved. Intact means the exact text
    // is here (`quote` at its confident score) or the structural fingerprint
    // is identical; an ambiguous quote, a fuzzy match, or a degraded
    // structural hit all mean the contents changed ⇒ drifted.
    let intact = matches!(quote, Some((_, s)) if s >= QUOTE_EXACT) || structural_intact;
    let status = if intact && range == saved {
        AnchorStatus::Anchored
    } else {
        AnchorStatus::Drifted { from: saved }
    };

    Resolution {
        range: Some(range),
        status,
        rungs,
    }
}
