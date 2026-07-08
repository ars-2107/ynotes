//! Query resolution: which notes a `file [LINE|START:END]` request returns,
//! and the never-drop guarantee around orphans.
//!
//! This is deliberately separate from anchoring. [`crate::anchor`] decides
//! *where* each note resolves; this module decides *which* resolved notes a
//! query should hand back, by intersecting their ranges with the queried
//! interval under each note's scope. Orphaned notes have no range to test, so
//! dropping them on a range query would silently lose context — instead they
//! are always returned for any query naming their file. That rule is the
//! product's core promise and is load-bearing; do not "optimise" it away.

use std::path::Path;
use std::str::FromStr;

use crate::anchor::{AnchorStatus, Resolution, resolve, resolve_for_relocation};
use crate::error::{Error, Result};
use crate::git::GitContext;
use crate::note::{Note, Scope};
use crate::source::{LineRange, SourceFile};
use crate::store::{MalformedNote, Store};

/// A parsed location argument: a single line, a range, or the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSpec {
    /// `230` — one line.
    Line(u32),
    /// `230:327` — an inclusive range.
    Range(LineRange),
    /// No location given — the whole file.
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

/// Parses one line number. Line numbers are 1-based, so `0` is rejected — it
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
/// past the last line is rejected as [`Error::InvalidLocation`] — the same
/// usage class as line `0` — because both are bad locations the caller chose,
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

/// A note paired with where (and how confidently) it resolved.
#[derive(Debug, Clone)]
pub struct ResolvedNote {
    /// The stored note.
    pub note: Note,
    /// Its resolution against the current file.
    pub resolution: Resolution,
    /// Set only when this note surfaced under a *different* file than its stored
    /// `target` because [`query`] detected the queried file was renamed from it
    /// — the pre-rename path the note is still filed under. `None` for a note
    /// resolved against its own target (every `list`/`lookup` note, and any
    /// query note that matched directly). Its presence tells an agent the note
    /// migrated here and a `reanchor` would make that durable.
    pub relocated_from: Option<String>,
}

/// The result of a query: notes that matched the interval, and the orphans
/// for the file (always included — never silently dropped).
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Notes whose resolved range satisfied the query under their scope.
    pub matched: Vec<ResolvedNote>,
    /// Notes for the file that no rung could locate.
    pub orphaned: Vec<ResolvedNote>,
    /// Records for the file the index pointed at but which could not be read
    /// or parsed. Skipped from `matched`/`orphaned` (they cannot be resolved)
    /// but surfaced here so a corrupt record is never silently dropped
    /// (invariant #4).
    pub malformed: Vec<MalformedNote>,
}

/// The result of a `list`/`lookup`: every resolved note, plus any records that
/// could not be read. The list-shaped analogue of [`QueryResult`]'s
/// `malformed`, carried so a store inventory cannot silently omit a corrupt
/// record (invariant #4).
#[derive(Debug, Clone)]
pub struct ListResult {
    /// Every note resolved against its current target.
    pub notes: Vec<ResolvedNote>,
    /// Records the index pointed at but which could not be read or parsed.
    pub malformed: Vec<MalformedNote>,
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
    let notes = scan.notes;

    // A deleted/absent file yields an empty source, so every rung misses and
    // the notes orphan — still returned, per the core promise.
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
        let resolution = resolve(&note.bundle, &source, git.as_ref());
        let resolved = ResolvedNote {
            note,
            resolution,
            relocated_from: None,
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
    })
}

/// Resolves every note in the store (optionally restricted to `only`),
/// returning each with its current anchor status. Unlike [`query`] this does
/// not filter by an interval — it is the backing for `ynotes list`.
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
/// [`Error::IndexMalformed`]/[`Error::IndexMissing`]/[`Error::Io`] if the
/// *index* is unreadable. A malformed *note* is surfaced in
/// [`ListResult::malformed`], never an error.
pub fn list(store: &Store, only: Option<&Path>) -> Result<ListResult> {
    let workdir = store.workdir()?.to_path_buf();
    let scan = match only {
        Some(p) if p.is_dir() => {
            // Directory filter: scan all notes and keep those under the
            // relativised prefix. A trailing slash anchors the boundary so
            // `auth/` does not collide with `authentication.js`. The store-
            // root itself (prefix == "") trivially matches every note —
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
            scan.malformed
                .retain(|m| prefix.is_empty() || m.target.starts_with(&prefix));
            scan
        }
        Some(p) => store.notes_for(&store.relativize(p)?)?,
        None => store.all_notes()?,
    };
    let git = GitContext::discover(&workdir);

    let mut cache: std::collections::HashMap<String, SourceFile> = std::collections::HashMap::new();
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
        let resolution = resolve(&note.bundle, source, git.as_ref());
        out.push(ResolvedNote {
            note,
            resolution,
            relocated_from: None,
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
    })
}

/// Resolve the stable `(target, body)` handle to the matching notes, each with
/// its current anchor status. The reader half of the stable-handle story: a
/// note's id rotates whenever it is re-anchored or updated (the id is
/// content-hashed over the bundle), and even a re-`save` after a commit rotates
/// it (the git selector advances), so an externally-recorded id goes stale.
/// `lookup` finds the live note(s) from the parts that persist — the target
/// path and the body text.
///
/// `target` filters by file or directory exactly as [`list`] does;
/// `body_contains`, when given, keeps only notes whose body contains the
/// substring (case-sensitive — code context is case-significant). With both
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
/// note that orphans against the current file is *not* surfaced — an orphan
/// does not belong to a file it cannot resolve in; its home stays the old path,
/// where a direct query still finds it.
///
/// Strictly read-only, so a query never writes (invariant #8). The migration is
/// made durable by `reanchor`, not here.
///
/// # Errors
///
/// [`Error::IndexMalformed`]/[`Error::IndexMissing`]/[`Error::Io`] if the index
/// or a note record for a pre-rename path cannot be read.
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
    for old_git_rel in ctx.renamed_from(&git_rel) {
        // git reports paths relative to the git root; translate back to a store
        // target via the absolute path. A pre-rename path outside the store
        // (`OutsideStore`) simply has no notes here — skip it.
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
            let resolution = resolve_for_relocation(&note.bundle, source);
            if matches!(resolution.status, AnchorStatus::Orphaned { .. }) {
                continue;
            }
            let resolved = ResolvedNote {
                note,
                resolution,
                relocated_from: Some(old_target.clone()),
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

/// Whether a resolved (non-orphan) note satisfies the query interval under
/// its scope. File-scoped notes always match; range/line-scoped notes match
/// on overlap (a whole-file query — `interval == None` — matches all).
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
