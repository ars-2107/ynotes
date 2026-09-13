//! Select notes by file and overlapping line range after anchor resolution.
//!
//! Orphans have no resolved range, so every query for their file includes them.
//! [`crate::anchor`] determines locations; this module applies query scope.

use std::path::Path;
use std::str::FromStr;

use crate::anchor::{AnchorStatus, Resolution, resolve, resolve_for_relocation};
use crate::error::{Error, Result};
use crate::git::{GitContext, RenameCache};
use crate::note::{Note, Scope};
use crate::source::{LineRange, SourceFile};
use crate::store::{IndexHealth, MalformedNote, Store};

/// Resolves a note and applies its independent review basis to the anchor
/// verdict.
///
/// The selector ladder answers whether it can locate the code represented by
/// the current selector bundle. The review basis answers a separate question:
/// whether that code is still what the note's author last reviewed. Keeping
/// those decisions separate lets `reanchor` refresh location selectors
/// without laundering stale context back to [`AnchorStatus::Anchored`].
#[must_use]
pub fn resolve_note(note: &Note, source: &SourceFile, git: Option<&GitContext>) -> Resolution {
    apply_review_basis(note, source, resolve(&note.bundle, source, git))
}

/// The rename-safe counterpart of [`resolve_note`].
pub(crate) fn resolve_note_for_relocation(note: &Note, source: &SourceFile) -> Resolution {
    apply_review_basis(note, source, resolve_for_relocation(&note.bundle, source))
}

fn apply_review_basis(note: &Note, source: &SourceFile, mut resolution: Resolution) -> Resolution {
    if let Some(range) = resolution.range {
        let reviewed = source
            .text(range)
            .is_ok_and(|text| note.review_matches(&text));
        if !reviewed {
            resolution.status = AnchorStatus::Drifted {
                from: note.bundle.position.range,
            };
        }
    }
    resolution
}

/// A parsed location argument: a single line, a range, or the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSpec {
    /// `230`, one line.
    Line(u32),
    /// `230:327`, an inclusive range.
    Range(LineRange),
    /// No location given, the whole file.
    Whole,
}

impl LineSpec {
    /// The interval this query restricts to, or `None` for a whole-file query
    /// (which matches every note regardless of position).
    #[must_use]
    pub fn interval(self) -> Option<LineRange> {
        match self {
            LineSpec::Line(n) => LineRange::new(n, n).ok(),
            LineSpec::Range(r) => Some(r),
            LineSpec::Whole => None,
        }
    }
}

impl FromStr for LineSpec {
    type Err = Error;

    /// Parses `"230"` or `"230:327"`. An empty string is [`LineSpec::Whole`].
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] on a component that is not a 1-based line number
    /// (non-numeric, or zero) or a reversed range; the binary maps this to a
    /// usage error (exit `2`).
    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Ok(LineSpec::Whole);
        }
        if let Some((a, b)) = s.split_once(':') {
            let start = parse_line(a)?;
            let end = parse_line(b)?;
            return Ok(LineSpec::Range(LineRange::new(start, end)?));
        }
        Ok(LineSpec::Line(parse_line(s)?))
    }
}

/// Parses one line number. Line numbers are 1-based, so `0` is rejected, it
/// is a usage error, not a silent whole-file query.
fn parse_line(tok: &str) -> Result<u32> {
    let tok = tok.trim();
    match tok.parse::<u32>() {
        Ok(0) => Err(Error::Invalid(format!(
            "line numbers are 1-based; `{tok}` is not a valid line"
        ))),
        Ok(n) => Ok(n),
        Err(_) => Err(Error::Invalid(format!("`{tok}` is not a line number"))),
    }
}

/// Maps a parsed location [`LineSpec`] to its scope and the [`LineRange`] to
/// capture, validated against `source`. A file note (`LineSpec::Whole`)
/// captures the whole file; a line or range captures exactly what it names.
///
/// The shared write-path helper behind `save`: it is where a location a caller
/// asked for is turned into the region a note is anchored to. A range running
/// past the last line is rejected as [`Error::InvalidLocation`], the same
/// usage class as line `0`, because both are bad locations the caller chose,
/// not runtime failures. Without this check a beyond-EOF range would only fail
/// deeper in the engine and read as a runtime error, inconsistent with the
/// line-`0` case a front-end already surfaces as usage.
///
/// # Errors
///
/// [`Error::InvalidLocation`] if `spec` is [`LineSpec::Whole`] over an empty
/// file, or names a line past the end of `source`.
pub fn resolve_scope(spec: LineSpec, source: &SourceFile) -> Result<(Scope, LineRange)> {
    let (scope, range) = match spec {
        LineSpec::Whole => {
            let n = source.line_count();
            if n == 0 {
                return Err(Error::InvalidLocation(
                    "cannot save a note for an empty file".to_owned(),
                ));
            }
            let r = LineRange::new(1, n).map_err(|e| Error::InvalidLocation(e.to_string()))?;
            (Scope::File, r)
        }
        LineSpec::Line(n) => {
            let r = LineRange::new(n, n).map_err(|e| Error::InvalidLocation(e.to_string()))?;
            (Scope::Line, r)
        }
        LineSpec::Range(r) => (Scope::Range, r),
    };

    let lines = source.line_count();
    if range.end() > lines {
        return Err(Error::InvalidLocation(format!(
            "line {} is past the end of the file ({lines} lines)",
            range.end()
        )));
    }
    Ok((scope, range))
}

/// Maps a location [`LineSpec`] plus the caller's quote of the region's text
/// (`code`) to its scope and the [`LineRange`] to capture, verifying the
/// location against `source` and correcting it when the coordinates have
/// gone stale.
///
/// The verified sibling of [`resolve_scope`]: the quote's first line is the
/// region's first line, `spec` is the caller's claim about where that is,
/// and the file is the authority. The rule, in order:
///
/// 1. the quote matches at the declared start → corroborated; the save
///    proceeds there, however many *other* places the quote occurs;
/// 2. the quote matches nowhere → [`Error::CodeNotFound`];
/// 3. the quote matches exactly once, elsewhere → the coordinates were
///    stale; the region relocates to the match;
/// 4. several matches, none at the declared start → [`Error::CodeAmbiguous`],
///    carrying every candidate start line.
///
/// Matching trims each line on both sides (indentation and trailing
/// whitespace are ignored, the text is not; interior blank lines stay
/// significant) and is otherwise **exact-or-refuse**. Fuzzy matching is a
/// resolve-time recovery mechanism: there it ships capped below the exact
/// rungs with an honest `drifted` verdict attached. A save has no verdict to
/// attach, a fuzzily mislocated save is captured into the selector bundle
/// as permanent ground truth that every later resolve will faithfully track.
/// Do not "improve" this path with the fuzzy rung.
///
/// The region's length is `end - start + 1` when `spec` declares an end,
/// else the quote's line count. A quote longer than a declared span still
/// matches in full, the overhang is a suffix fingerprint that pins down
/// *where* the region is, never *what* it spans. Blank edge lines of the
/// quote are dropped; leading ones advance the declared start (and shrink a
/// declared span), so a truthful quote that opens blank corroborates instead
/// of spuriously relocating.
///
/// # Errors
///
/// [`Error::InvalidLocation`] if `spec` is [`LineSpec::Whole`] (`code`
/// requires a starting line), the quote is empty or blank, the quote's blank
/// lead pushes the start past a declared end, or the resolved region runs
/// past the end of `source`; [`Error::CodeNotFound`] and
/// [`Error::CodeAmbiguous`] per the rule above.
pub fn resolve_scope_verified(
    spec: LineSpec,
    code: &str,
    source: &SourceFile,
) -> Result<(Scope, LineRange)> {
    let (declared_start, declared_end) = match spec {
        LineSpec::Whole => {
            return Err(Error::InvalidLocation(
                "`code` requires a starting line".to_owned(),
            ));
        }
        LineSpec::Line(n) => (n, None),
        LineSpec::Range(r) => (r.start(), Some(r.end())),
    };
    // `LineSpec::Line(0)` is constructible even though the parser rejects it;
    // guard here so the 0-based membership test below cannot underflow.
    if declared_start == 0 {
        return Err(Error::InvalidLocation(
            "line numbers are 1-based; `0` is not a valid line".to_owned(),
        ));
    }

    // Normalise the quote exactly as the file is normalised below: trim each
    // line on both sides. Blank *edge* lines are dropped (callers quote with
    // sloppy edges); interior blanks stay significant. `str::lines()` splits
    // the quote the same way `SourceFile::read` split the file, so a CRLF/LF
    // mismatch needs no special handling.
    let quote: Vec<&str> = code.lines().map(str::trim).collect();
    let lead = quote.iter().take_while(|l| l.is_empty()).count();
    let trail = quote.iter().rev().take_while(|l| l.is_empty()).count();
    if lead + trail >= quote.len() {
        return Err(Error::InvalidLocation(
            "`code` is empty; quote the region's text as the file reads now".to_owned(),
        ));
    }
    let needle = &quote[lead..quote.len() - trail];

    // `start` names the quote's first line as the caller wrote it, so the
    // dropped blank lead advances it (and shrinks a declared span). All
    // arithmetic in u64: a caller-supplied `end` of `u32::MAX` must report
    // past-EOF below, not overflow here.
    let start = u64::from(declared_start) + u64::try_from(lead).unwrap_or(u64::MAX);
    let span = match declared_end {
        Some(end) => {
            let end = u64::from(end);
            if end < start {
                return Err(Error::InvalidLocation(format!(
                    "the quote's leading blank lines advance start to {start}, past end {end}"
                )));
            }
            end - start + 1
        }
        None => u64::try_from(needle.len()).unwrap_or(u64::MAX),
    };
    // Scope keeps resolve_scope's wart intact: `Line(n)` is a line note only
    // when the quote is one line; a declared range is always a range note,
    // even a degenerate `n:n`.
    let scope = match spec {
        LineSpec::Line(_) if needle.len() == 1 => Scope::Line,
        _ => Scope::Range,
    };

    // One scan for every match of the trimmed quote in the trimmed file.
    // `str::trim` borrows, so the haystack is a Vec of subslices, no
    // per-line allocation. A needle longer than the file yields no windows,
    // which falls out as CodeNotFound below.
    let hay: Vec<&str> = source.lines_slice().iter().map(|l| l.trim()).collect();
    let matches: Vec<usize> = hay
        .windows(needle.len())
        .enumerate()
        .filter_map(|(i, w)| (w == needle).then_some(i))
        .collect();

    let start0 = start - 1;
    let at_start = matches
        .iter()
        .any(|&m| u64::try_from(m).unwrap_or(u64::MAX) == start0);
    let origin = if at_start {
        // Corroborated: the caller's coordinates check out, and collisions
        // elsewhere are irrelevant, this is what keeps duplicate-heavy
        // files saveable at all (spec §4.4).
        start
    } else {
        match matches[..] {
            [] => return Err(Error::CodeNotFound),
            [only] => u64::try_from(only).unwrap_or(u64::MAX) + 1,
            _ => {
                return Err(Error::CodeAmbiguous {
                    lines: matches
                        .iter()
                        .map(|&m| u32::try_from(m + 1).unwrap_or(u32::MAX))
                        .collect(),
                });
            }
        }
    };

    let end = origin + span - 1;
    let lines = u64::from(source.line_count());
    if end > lines {
        // When the quote moved the region, `end` is a number the caller never
        // passed, without the match line the message reads as noise (a
        // declared `2:9` surfacing as "line 11"). Name where the quote
        // matched so the correction is one step: re-declare from that line.
        let msg = if origin == start {
            format!("line {end} is past the end of the file ({lines} lines)")
        } else {
            format!(
                "the quote matched at line {origin}, but the declared span ends at line {end}, \
                 past the end of the file ({lines} lines)"
            )
        };
        return Err(Error::InvalidLocation(msg));
    }
    // Both bounds are >= 1 and <= `lines` <= `u32::MAX`: exact conversions.
    let range = LineRange::new(
        u32::try_from(origin).unwrap_or(u32::MAX),
        u32::try_from(end).unwrap_or(u32::MAX),
    )
    .map_err(|e| Error::InvalidLocation(e.to_string()))?;
    Ok((scope, range))
}

/// A note paired with where (and how confidently) it resolved.
#[derive(Debug, Clone)]
pub struct ResolvedNote {
    /// The stored note.
    pub note: Note,
    /// Its resolution against the current file.
    pub resolution: Resolution,
    /// Set only when this note resolved under a *different* file than its stored
    /// `target` because git proposed a rename and the anchor ladder corroborated
    /// its content. This is the pre-rename path the note is still filed under.
    /// Its presence tells an agent that `reanchor` would make the migration
    /// durable.
    pub relocated_from: Option<String>,
    /// The current path. Set on every read that sets [`Self::relocated_from`],
    /// so a pending migration always names both of its sides. The pair carries
    /// the whole fact on an inventory read, where nothing in the request says
    /// which file a note resolved against; on a file query the destination is
    /// simply the path that was asked about, so this restates it rather than
    /// adding to it. Filling either side persists nothing.
    pub relocated_to: Option<String>,
}

/// The result of a query: notes that matched the interval, and the orphans
/// for the file (always included, never silently dropped).
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Notes whose resolved range satisfied the query under their scope.
    pub matched: Vec<ResolvedNote>,
    /// Notes for the file that no rung could locate.
    pub orphaned: Vec<ResolvedNote>,
    /// Records relevant to the file that could not be read
    /// or parsed. Skipped from `matched`/`orphaned` (they cannot be resolved)
    /// but surfaced here so a corrupt record is never silently dropped
    /// (invariant #4).
    pub malformed: Vec<MalformedNote>,
    /// Drift observed between `notes/` and the rebuildable index cache while
    /// serving this read. The read does not repair it.
    pub index: IndexHealth,
}

/// The result of a `list`/`lookup`: every resolved note, plus any records that
/// could not be read. The list-shaped analogue of [`QueryResult`]'s
/// `malformed`, carried so a store inventory cannot silently omit a corrupt
/// record (invariant #4).
#[derive(Debug, Clone)]
pub struct ListResult {
    /// Every note resolved against its stored target or a content-corroborated
    /// staged or committed rename destination. A pending rename is read-only and
    /// carries both relocation paths.
    pub notes: Vec<ResolvedNote>,
    /// Records under `notes/` that could not be read or parsed.
    pub malformed: Vec<MalformedNote>,
    /// Drift observed between `notes/` and the rebuildable index cache while
    /// serving this inventory. The read does not repair it.
    pub index: IndexHealth,
}

/// Resolves every note for `file` and returns those a `at` query should yield.
///
/// `file` is interpreted relative to the store's work tree. A missing file is
/// not an error: with no content to match, notes simply orphan and are still
/// surfaced.
///
/// # Errors
///
/// [`Error::Invalid`] if `file` is outside the store, or [`Error::Io`] /
/// [`Error::Invalid`] if the index or a note record cannot be read.
pub fn query(store: &Store, file: &Path, at: LineSpec) -> Result<QueryResult> {
    let target = store.relativize(file)?;
    let scan = store.notes_for(&target)?;
    let index = scan.index;
    let notes = scan.notes;

    // A deleted/absent file yields an empty source, so every rung misses and
    // the notes orphan, still returned, per the core promise.
    let source = match SourceFile::read(file) {
        Ok(s) => s,
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            SourceFile::from_lines(file.to_path_buf(), Vec::new())
        }
        Err(e) => return Err(e),
    };

    let git_dir = file.parent().unwrap_or_else(|| Path::new("."));
    let git = GitContext::discover(git_dir);

    let interval = at.interval();
    let mut matched = Vec::new();
    let mut orphaned = Vec::new();

    let mut returned_ids = std::collections::BTreeSet::new();

    for note in notes {
        let resolution = resolve_note(&note, &source, git.as_ref());
        let resolved = ResolvedNote {
            note,
            resolution,
            relocated_from: None,
            relocated_to: None,
        };

        match resolved.resolution.status {
            AnchorStatus::Orphaned { .. } => {
                returned_ids.insert(resolved.note.id.clone());
                orphaned.push(resolved);
            }
            _ => {
                if includes(&resolved, interval) {
                    returned_ids.insert(resolved.note.id.clone());
                    matched.push(resolved);
                }
            }
        }
    }

    // Read-side self-heal: a file may have been renamed from a path that still
    // has notes. Reverse-detect that rename and surface those notes here,
    // resolved against the current file, so an agent working at the new path
    // finds them before any `reanchor` has migrated them. This must run even
    // when the new path already has notes of its own; otherwise a fresh note on
    // the destination hides all pre-rename context until maintenance runs.
    // Read-only (invariant #8).
    if let Some(ctx) = git.as_ref() {
        reverse_detect(
            store,
            ctx,
            file,
            &source,
            interval,
            &mut matched,
            &mut returned_ids,
        )?;
    }

    matched.sort_by(|a, b| {
        scope_rank(a.note.scope)
            .cmp(&scope_rank(b.note.scope))
            .then(range_start(a).cmp(&range_start(b)))
    });

    Ok(QueryResult {
        matched,
        orphaned,
        malformed: scan.malformed,
        index,
    })
}

/// One annotated file: its target path and how many notes it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotatedFile {
    /// The target path, store-relative and forward-slashed.
    pub target: String,
    /// How many notes are discoverable at it, including pending renames.
    pub notes: usize,
}

/// Which files in the store carry notes, and how many each has.
#[derive(Debug, Clone)]
pub struct FileInventory {
    /// Annotated files, most-annotated first (ties broken by path).
    pub files: Vec<AnnotatedFile>,
    /// Total notes across every file, the sum of `files[].notes`.
    pub total: usize,
    /// Records under `notes/` that could not be read or parsed.
    pub malformed: usize,
    /// Drift between the record files and the rebuildable index cache.
    pub index: IndexHealth,
}

/// Which files carry notes, following corroborated renames of missing targets.
///
/// Existing targets need only records and a path-existence check. Missing
/// targets use Git rename detection and content corroboration, shared with
/// [`query`], so orientation does not strand notes under an obsolete filename.
/// An unlocatable note retains its historical path. No records are rewritten.
/// Use [`query`] or [`list`] for ranges and statuses.
///
/// Records stay authoritative (invariant #12): the walk recovers notes the
/// index omitted and reports cache drift without repairing it.
///
/// # Errors
///
/// [`Error::Io`] if record enumeration fails. Malformed records are counted,
/// never an error.
pub fn files(store: &Store) -> Result<FileInventory> {
    let scan = store.all_notes()?;
    let workdir = store.workdir()?;
    let git = scan
        .notes
        .iter()
        .any(|note| !workdir.join(&note.target).exists())
        .then(|| GitContext::discover(workdir))
        .flatten();
    let mut cache = std::collections::HashMap::new();
    let mut renames = RenameCache::default();
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for note in &scan.notes {
        // A missing stored path may be a pending rename. Use the same
        // content corroboration as query, never Git's filename guess alone.
        let destination = if workdir.join(&note.target).exists() {
            None
        } else {
            git.as_ref().and_then(|ctx| {
                resolve_forward_rename(store, ctx, note, &mut cache, &mut renames)
                    .map(|(target, _)| target)
            })
        };
        *counts
            .entry(destination.unwrap_or_else(|| note.target.clone()))
            .or_default() += 1;
    }
    let mut files: Vec<AnnotatedFile> = counts
        .into_iter()
        .map(|(target, notes)| AnnotatedFile { target, notes })
        .collect();
    // Most-annotated first so a caller that truncates keeps the files most
    // worth knowing about; the path tiebreak keeps the order total, so the same
    // store always reports identically.
    files.sort_by(|a, b| b.notes.cmp(&a.notes).then_with(|| a.target.cmp(&b.target)));
    Ok(FileInventory {
        total: scan.notes.len(),
        malformed: scan.malformed.len(),
        index: scan.index,
        files,
    })
}

/// Resolves every note in the store (optionally restricted to `only`),
/// returning each with its current anchor status. Unlike [`query`] this does
/// not filter by an interval, it is the backing for `ynotes list`.
///
/// `only` may name a file *or* a directory: a directory restricts the list
/// to notes whose `target` sits anywhere beneath it (prefix match on the
/// store-relative path with a `/` boundary, so `auth` does not match
/// `authentication`).
///
/// Reads each target file once (cached) and discovers git once from the work
/// tree, so listing a whole repo is cheap.
///
/// # Errors
///
/// [`Error::Invalid`] if `only` is outside the store, or
/// [`Error::Io`] if record enumeration fails. Missing/malformed index state is
/// reported in [`ListResult::index`], and malformed notes are surfaced in
/// [`ListResult::malformed`], never an error.
pub fn list(store: &Store, only: Option<&Path>) -> Result<ListResult> {
    let workdir = store.workdir()?.to_path_buf();
    let scan = match only {
        Some(p) if p.is_dir() => {
            // Directory filter: scan all notes and keep those under the
            // relativised prefix. A trailing slash anchors the boundary so
            // `auth/` does not collide with `authentication.js`. The store-
            // root itself (prefix == "") trivially matches every note,
            // equivalent to `list` with no filter.
            let rel = store.relativize(p)?;
            let prefix = if rel.is_empty() {
                String::new()
            } else {
                format!("{rel}/")
            };
            let mut scan = store.all_notes()?;
            // Filter both the healthy and the malformed records by the same
            // prefix, so a directory listing's `malformed` reports only records
            // under that directory (each malformed record carries its target).
            scan.notes
                .retain(|n| prefix.is_empty() || n.target.starts_with(&prefix));
            scan.malformed.retain(|m| {
                prefix.is_empty()
                    || m.target
                        .as_ref()
                        .is_none_or(|target| target.starts_with(&prefix))
            });
            scan
        }
        Some(p) => store.notes_for(&store.relativize(p)?)?,
        None => store.all_notes()?,
    };
    let git = GitContext::discover(&workdir);

    let mut cache: std::collections::HashMap<String, SourceFile> = std::collections::HashMap::new();
    let mut renames = RenameCache::default();
    let mut out = Vec::with_capacity(scan.notes.len());
    for note in scan.notes {
        if !cache.contains_key(&note.target) {
            let abs = workdir.join(&note.target);
            // Missing/unreadable target ⇒ empty source ⇒ the note orphans
            // and is still surfaced (never dropped).
            let s =
                SourceFile::read(&abs).unwrap_or_else(|_| SourceFile::from_lines(abs, Vec::new()));
            cache.insert(note.target.clone(), s);
        }
        let source = &cache[&note.target];
        let resolution = resolve_note(&note, source, git.as_ref());
        let relocation = matches!(resolution.status, AnchorStatus::Orphaned { .. })
            .then(|| {
                git.as_ref().and_then(|ctx| {
                    resolve_forward_rename(store, ctx, &note, &mut cache, &mut renames)
                })
            })
            .flatten();
        let (resolution, relocated_to) =
            relocation.map_or((resolution, None), |(target, found)| (found, Some(target)));
        let relocated_from = relocated_to.as_ref().map(|_| note.target.clone());
        out.push(ResolvedNote {
            note,
            resolution,
            relocated_from,
            relocated_to,
        });
    }

    out.sort_by(|a, b| {
        a.note
            .target
            .cmp(&b.note.target)
            .then(scope_rank(a.note.scope).cmp(&scope_rank(b.note.scope)))
            .then(range_start(a).cmp(&range_start(b)))
    });
    Ok(ListResult {
        notes: out,
        malformed: scan.malformed,
        index: scan.index,
    })
}

/// Resolve a single note exactly as [`list`] resolves each of its own,
/// including the rename corroboration.
///
/// By-ID reads and inventory share this policy so a pending rename has the
/// same status regardless of how the note was selected.
///
/// # Errors
///
/// Propagates the store's work-tree lookup. An unreadable target is *not* an
/// error: it resolves `orphaned` and is still surfaced (invariant #4).
pub fn resolve_one(store: &Store, note: Note) -> Result<ResolvedNote> {
    let workdir = store.workdir()?.to_path_buf();
    let git = GitContext::discover(&workdir);

    let mut cache: std::collections::HashMap<String, SourceFile> = std::collections::HashMap::new();
    let abs = workdir.join(&note.target);
    // Missing/unreadable target ⇒ empty source ⇒ the note orphans and is
    // still surfaced (never dropped).
    let source = SourceFile::read(&abs).unwrap_or_else(|_| SourceFile::from_lines(abs, Vec::new()));
    cache.insert(note.target.clone(), source);

    let resolution = resolve_note(&note, &cache[&note.target], git.as_ref());
    let mut renames = RenameCache::default();
    let relocation = matches!(resolution.status, AnchorStatus::Orphaned { .. })
        .then(|| {
            git.as_ref()
                .and_then(|ctx| resolve_forward_rename(store, ctx, &note, &mut cache, &mut renames))
        })
        .flatten();
    let (resolution, relocated_to) =
        relocation.map_or((resolution, None), |(target, found)| (found, Some(target)));
    let relocated_from = relocated_to.as_ref().map(|_| note.target.clone());
    Ok(ResolvedNote {
        note,
        resolution,
        relocated_from,
        relocated_to,
    })
}

/// Resolve the stable `(target, body)` handle to the matching notes, each with
/// its current anchor status. The reader half of the stable-handle story: a
/// note's id rotates whenever its anchor, body, or review basis changes (the id is
/// content-hashed over the bundle), and even a re-`save` after a commit rotates
/// it (the git selector advances), so an externally-recorded id goes stale.
/// `lookup` finds the live note(s) from the parts that persist, the target
/// path and the body text.
///
/// `target` filters by file or directory exactly as [`list`] does;
/// `body_contains`, when given, keeps only notes whose body contains the
/// substring (case-sensitive, code context is case-significant). With both
/// `None` the result is every note in the store; the binary requires at least
/// one filter, but the engine stays permissive for programmatic callers.
///
/// Read-only: like [`list`] and [`query`] it resolves but never writes
/// (invariant #8).
///
/// # Errors
///
/// As [`list`]: [`Error::Invalid`] if `target` is outside the store, or
/// [`Error::Io`] / [`Error::Invalid`] if the index or a note record cannot be
/// read.
pub fn lookup(
    store: &Store,
    target: Option<&Path>,
    body_contains: Option<&str>,
) -> Result<ListResult> {
    let mut result = list(store, target)?;
    if let Some(needle) = body_contains {
        result.notes.retain(|rn| rn.note.body.contains(needle));
    }
    Ok(result)
}

/// Surface notes stored under a file's *pre-rename* path: the read-side half
/// of following a rename.
///
/// Asks git which earlier paths the current file was renamed from, loads the
/// notes filed under each, resolves them against the current file, and pushes
/// the **located** ones (interval-filtered) into `matched`, tagged with the
/// pre-rename `target` in [`ResolvedNote::relocated_from`]. A reverse-detected
/// note that orphans against the current file is *not* surfaced, an orphan
/// does not belong to a file it cannot resolve in; its home stays the old path,
/// where a direct query still finds it.
///
/// Strictly read-only, so a query never writes (invariant #8). The migration is
/// made durable by `reanchor`, not here.
///
/// # Errors
///
/// [`Error::Io`] if record enumeration for a pre-rename path fails. Rebuildable
/// index failures degrade to authoritative record reads.
fn reverse_detect(
    store: &Store,
    ctx: &GitContext,
    file: &Path,
    source: &SourceFile,
    interval: Option<LineRange>,
    matched: &mut Vec<ResolvedNote>,
    returned_ids: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    let Some(git_rel) = ctx.relativize(file) else {
        return Ok(());
    };
    let current_target = store.relativize(file)?;
    for old_git_rel in ctx.renamed_from(&git_rel) {
        // git reports paths relative to the git root; translate back to a store
        // target via the absolute path. A pre-rename path outside the store
        // (`OutsideStore`) simply has no notes here, skip it.
        let old_abs = ctx.root().join(&old_git_rel);
        let Ok(old_target) = store.relativize(&old_abs) else {
            continue;
        };
        for note in store.notes_for(&old_target)?.notes {
            // Corroborate in relocation mode (`resolve_for_relocation`): content
            // rungs only (the note's git selector names the renamed-away old
            // path, so the git rung would transport onto the deletion point and
            // false-witness a moved fuzzy match, surfacing the note against
            // unrelated code), and with the "did not move" shortcut dropped (a
            // fuzzy hit at the saved line numbers is meaningless in this
            // different file). Invariant #10; mirrors `reanchor`'s relocation
            // corroboration exactly.
            let resolution = resolve_note_for_relocation(&note, source);
            if matches!(resolution.status, AnchorStatus::Orphaned { .. }) {
                continue;
            }
            let resolved = ResolvedNote {
                note,
                resolution,
                relocated_from: Some(old_target.clone()),
                relocated_to: Some(current_target.clone()),
            };
            if includes(&resolved, interval) {
                if !returned_ids.insert(resolved.note.id.clone()) {
                    continue;
                }
                matched.push(resolved);
            }
        }
    }
    Ok(())
}

/// Resolve a stored note against the path git says its original target was
/// renamed to, without persisting the move.
///
/// The note's own baseline drives the forward lookup, so this works for both
/// staged and committed renames and does not require a reverse history walk.
/// The destination is accepted only when the relocation-mode content ladder
/// corroborates the region there, matching reanchor's wrong-file guard.
fn resolve_forward_rename(
    store: &Store,
    ctx: &GitContext,
    note: &Note,
    cache: &mut std::collections::HashMap<String, SourceFile>,
    renames: &mut RenameCache,
) -> Option<(String, Resolution)> {
    let git = note.bundle.git.as_ref()?;
    let new_git_rel = renames.lookup(ctx, &git.commit, &git.path)?;
    let new_abs = ctx.root().join(new_git_rel);
    let new_target = store.relativize(&new_abs).ok()?;
    if !cache.contains_key(&new_target) {
        cache.insert(new_target.clone(), SourceFile::read(&new_abs).ok()?);
    }
    let resolution = resolve_note_for_relocation(note, &cache[&new_target]);
    (!matches!(resolution.status, AnchorStatus::Orphaned { .. }))
        .then_some((new_target, resolution))
}

/// Whether a resolved (non-orphan) note satisfies the query interval under
/// its scope. File-scoped notes always match; range/line-scoped notes match
/// on overlap (a whole-file query, `interval == None`, matches all).
fn includes(rn: &ResolvedNote, interval: Option<LineRange>) -> bool {
    if rn.note.scope == Scope::File {
        return true;
    }
    match (interval, rn.resolution.range) {
        (None, _) => true,
        (Some(q), Some(r)) => q.overlaps(r),
        (Some(_), None) => false,
    }
}

/// File-scoped notes sort first, then range, then line.
fn scope_rank(scope: Scope) -> u8 {
    match scope {
        Scope::File => 0,
        Scope::Range => 1,
        Scope::Line => 2,
    }
}

/// Resolved start line for sorting (0 when unresolved).
fn range_start(rn: &ResolvedNote) -> u32 {
    rn.resolution.range.map_or(0, LineRange::start)
}
