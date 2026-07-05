//! R1's view of git: a thin shell-out wrapper, deliberately *not* a git
//! library.
//!
//! Why shell out rather than depend on `gix`: `gix` forces `Zlib` and
//! `BSD-3-Clause` into this project's deliberately narrow `cargo deny`
//! licence allow-list. Invoking the `git` binary and parsing its porcelain
//! adds zero Rust dependencies and zero
//! licence-policy change. The cost — a runtime dependency on a `git`
//! executable — is absorbed by the ladder: if git is absent or the file is
//! not tracked, R1 reports [`Skipped`](crate::anchor::RungResult::Skipped)
//! and the other rungs carry the resolution. R1 never turns a missing `git`
//! into an error, so a note is never lost to the absence of a tool.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::path::to_forward_slash;
use crate::source::LineRange;

/// A discovered git repository: its work-tree root.
#[derive(Debug, Clone)]
pub struct GitContext {
    root: PathBuf,
}

/// The outcome of transporting a saved line range to the working file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// The file is unchanged since the saved commit; the range still holds.
    Unchanged(LineRange),
    /// The file changed but the range was carried through the diff. The
    /// region may have shifted (and possibly been edited inside).
    Moved {
        /// The range translated into the current file.
        to: LineRange,
        /// Whether the saved range overlapped a changed hunk (so its
        /// *contents*, not just its position, may differ).
        touched: bool,
    },
    /// git could answer, but not about this range/path (path absent at the
    /// saved commit, or deleted in the work tree). R1 contributes nothing.
    Unknown,
}

impl GitContext {
    /// Discovers the repository containing `near`, or returns `None` if `near`
    /// is not in a git work tree or no `git` binary is available. Never errors
    /// — absence of git is a normal, non-fatal state for the ladder.
    #[must_use]
    pub fn discover(near: &Path) -> Option<Self> {
        let out = run(near, &["rev-parse", "--show-toplevel"])?;
        let root = PathBuf::from(out.trim());
        root.is_dir().then_some(Self { root })
    }

    /// The work-tree root of the discovered repository.
    ///
    /// Rename detection reports paths relative to *this* root, which is not
    /// necessarily the ynotes store root (a `.ynotes` may sit in a
    /// subdirectory of the repo). A caller translating a git-relative path back
    /// to a store target needs this base to rebuild the absolute path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The current `HEAD` commit id, or `None` if it cannot be read (e.g. an
    /// empty repository with no commits yet).
    #[must_use]
    pub fn head_commit(&self) -> Option<String> {
        run(&self.root, &["rev-parse", "HEAD"]).map(|s| s.trim().to_owned())
    }

    /// `path` expressed relative to the work-tree root with forward slashes,
    /// or `None` if `path` is outside the repository.
    #[must_use]
    pub fn relativize(&self, path: &Path) -> Option<String> {
        let abs = path.canonicalize().ok()?;
        let root = self.root.canonicalize().ok()?;
        let rel = abs.strip_prefix(&root).ok()?;
        Some(to_forward_slash(rel))
    }

    /// Transports `range` from `from_commit` (at repo-relative `rel_path`) to
    /// the current working file, by diffing the commit against the work tree
    /// and translating line numbers through the hunks.
    ///
    /// Returns `Transport::Unknown` if git cannot diff that path (so R1
    /// stays silent rather than guessing).
    #[must_use]
    pub fn transport(&self, from_commit: &str, rel_path: &str, range: LineRange) -> Transport {
        // `--unified=0` keeps hunks minimal so the line map is tight;
        // `-M` follows a same-content rename of this path.
        let Some(diff) = run(
            &self.root,
            &[
                "diff",
                "--no-color",
                "--unified=0",
                "-M",
                from_commit,
                "--",
                rel_path,
            ],
        ) else {
            return Transport::Unknown;
        };

        if diff.trim().is_empty() {
            // No diff for this path between the commit and the work tree: the
            // region is exactly where it was saved.
            return Transport::Unchanged(range);
        }

        let hunks = parse_hunks(&diff);
        translate(range, &hunks)
    }

    /// Whether `rel_path` exists as a blob in `commit`'s tree — i.e. whether
    /// the file was tracked at that commit.
    ///
    /// `git cat-file -e <commit>:<rel_path>` exits zero iff that object
    /// exists, so the `run` helper yielding `Some` is exactly the "tracked"
    /// answer.
    /// Absence of the path — and any failure, including no `git` binary —
    /// answers `false`: an untracked file has no meaningful git baseline, so
    /// the caller must not attach a git selector for it.
    #[must_use]
    pub fn tracks(&self, commit: &str, rel_path: &str) -> bool {
        run(
            &self.root,
            &["cat-file", "-e", &format!("{commit}:{rel_path}")],
        )
        .is_some()
    }

    /// Expresses a working-tree line `range` in the coordinates of `commit`'s
    /// version of `rel_path` — the inverse of [`transport`](Self::transport).
    ///
    /// `save` and `reanchor` capture a region by its *working-tree* range,
    /// while the git rung's baseline is a *commit*. When the working tree is
    /// dirty those coordinate systems differ, so the captured range is
    /// translated back into the commit before it is stored — otherwise every
    /// later transport carries the dirty-diff skew. Best-effort: if git
    /// cannot diff that path the `range` is returned unchanged, and the git
    /// rung simply degrades like any other absent signal.
    #[must_use]
    pub fn range_in_commit(&self, commit: &str, rel_path: &str, range: LineRange) -> LineRange {
        // `-R` reverses the diff so its hunks map the work tree (old side) to
        // `commit` (new side); `translate` then carries `range` that way.
        let Some(diff) = run(
            &self.root,
            &[
                "diff",
                "--no-color",
                "--unified=0",
                "-M",
                "-R",
                commit,
                "--",
                rel_path,
            ],
        ) else {
            return range;
        };
        if diff.trim().is_empty() {
            return range;
        }
        match translate(range, &parse_hunks(&diff)) {
            Transport::Moved { to, .. } => to,
            Transport::Unchanged(r) => r,
            Transport::Unknown => range,
        }
    }

    /// Where the file tracked at `rel_path` in `from_commit` now lives in the
    /// work tree, if git detects it was renamed since — `None` when it was not
    /// renamed (still present, or genuinely gone).
    ///
    /// Compares `from_commit`'s tree against the work tree with rename
    /// detection, so it catches both a **committed** and a **staged** rename in
    /// one call. The diff is filtered to renames and left unscoped: a pathspec
    /// on the *old* path suppresses git's rename pairing (the new path is then
    /// excluded from the diff), so the whole rename set is fetched and matched
    /// on the old side here.
    ///
    /// A plain unstaged `mv` to an *untracked* path is invisible to
    /// `git diff` (the new file is untracked) and correctly yields `None`.
    /// Best-effort: any git failure yields `None`, so a missing tool never
    /// turns into an error.
    #[must_use]
    pub fn renamed_to(&self, from_commit: &str, rel_path: &str) -> Option<String> {
        let out = run(
            &self.root,
            &[
                "diff",
                RENAME_FIND,
                "--name-status",
                "-z",
                "--diff-filter=R",
                from_commit,
            ],
        )?;
        parse_renames_z(&out)
            .into_iter()
            .find(|(old, _)| old == rel_path)
            .map(|(_, new)| new)
    }

    /// Every earlier path the work-tree file `rel_path` was renamed from, most
    /// recent first and de-duplicated — the reverse of
    /// [`renamed_to`](Self::renamed_to).
    ///
    /// Two sources are unioned so the reverse view is as complete as the
    /// forward one:
    ///
    /// - the **uncommitted** diff (`HEAD` against the work tree), so a
    ///   staged-but-not-yet-committed `git mv` is seen — git *history* cannot
    ///   show it because it is not in a commit yet;
    /// - the **committed history** via `git log --follow`, git's own
    ///   rename-following machinery, so a multi-hop chain (`a` → `b` →
    ///   `rel_path`) yields every prior name.
    ///
    /// Empty when the file has no rename history, is untracked, or git is
    /// unavailable — never an error.
    #[must_use]
    pub fn renamed_from(&self, rel_path: &str) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut names = Vec::new();

        // Uncommitted (staged or working-tree) rename into `rel_path`. Matched
        // on the *new* side, since we are asking where this current path came
        // from.
        if let Some(out) = run(
            &self.root,
            &[
                "diff",
                RENAME_FIND,
                "--name-status",
                "-z",
                "--diff-filter=R",
                "HEAD",
            ],
        ) {
            for (old, new) in parse_renames_z(&out) {
                if new == rel_path && seen.insert(old.clone()) {
                    names.push(old);
                }
            }
        }

        // Committed history. `--follow` scopes to this single path and yields
        // the old side of every rename step in its chain.
        if let Some(out) = run(
            &self.root,
            &[
                "log",
                "--follow",
                RENAME_FIND,
                "--name-status",
                "-z",
                "--diff-filter=R",
                "--format=",
                "--",
                rel_path,
            ],
        ) {
            for (old, _new) in parse_renames_z(&out) {
                if seen.insert(old.clone()) {
                    names.push(old);
                }
            }
        }
        names
    }
}

/// git's rename-similarity threshold, passed as `-M<n>%`. Lowered from git's
/// 50% default because a small file renamed *and* edited in one step can score
/// well under 50% (a few changed characters weigh heavily in a short blob), and
/// missing that rename would orphan the note. A generous threshold is safe here
/// precisely because git's signal is never trusted alone: every proposed rename
/// is corroborated by the content ladder at the new path (invariant #10), so a
/// spurious pairing resolves to no region and is declined rather than acted on.
const RENAME_FIND: &str = "-M40%";

/// Parses `-z --name-status` output, returning `(old, new)` rename pairs.
///
/// Git's non-`-z` porcelain C-quotes paths containing characters like quotes,
/// tabs, newlines, or backslashes. `-z` leaves path bytes unquoted and
/// separates fields with NUL, which gives this parser an unambiguous boundary.
/// Non-rename records are ignored; only a rename moves a note.
fn parse_renames_z(name_status: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut fields = name_status.split('\0').filter(|field| !field.is_empty());
    while let Some(status) = fields.next() {
        if status.starts_with('R') {
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                break;
            };
            out.push((old.to_owned(), new.to_owned()));
            continue;
        }
        // Defensive for callers that ever relax `--diff-filter=R`: copies also
        // carry two path fields, while ordinary records carry one.
        if status.starts_with('C') {
            let _ = fields.next();
            let _ = fields.next();
        } else {
            let _ = fields.next();
        }
    }
    out
}

/// The version of the `git` executable on `PATH` (the `2.43.0` in
/// `git version 2.43.0`), or `None` if no `git` binary is available.
///
/// R1's transport rung shells out to `git` and silently degrades to
/// [`Skipped`](crate::anchor::RungResult::Skipped) when it is absent — by
/// design, so a note is never lost to a missing tool. This lets the `doctor`
/// front-end *report* that degradation instead of leaving a user to deduce it
/// from weaker-than-expected anchors.
#[must_use]
pub fn git_version() -> Option<String> {
    let output = Command::new("git").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(
        text.trim()
            .strip_prefix("git version ")
            .unwrap_or(text.trim())
            .to_owned(),
    )
}

/// One unified-diff hunk header: `@@ -old_start,old_len +new_start,new_len @@`.
#[derive(Debug, Clone, Copy)]
struct Hunk {
    old_start: u32,
    old_len: u32,
    new_start: u32,
    new_len: u32,
}

/// Parses every `@@ ... @@` header out of a unified diff. A missing length
/// (`@@ -5 +6 @@`) means a length of one, per the diff format.
fn parse_hunks(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    for line in diff.lines().filter(|l| l.starts_with("@@")) {
        // line looks like: @@ -a,b +c,d @@ optional section heading
        let mut parts = line.split_whitespace();
        let (_at, old, new) = (parts.next(), parts.next(), parts.next());
        let (Some(old), Some(new)) = (old, new) else {
            continue;
        };
        let parse_side = |s: &str| -> Option<(u32, u32)> {
            let s = s.trim_start_matches(['-', '+']);
            let mut it = s.split(',');
            let start: u32 = it.next()?.parse().ok()?;
            let len: u32 = it.next().map_or(Some(1), |v| v.parse().ok())?;
            Some((start, len))
        };
        if let (Some((os, ol)), Some((ns, nl))) = (parse_side(old), parse_side(new)) {
            hunks.push(Hunk {
                old_start: os,
                old_len: ol,
                new_start: ns,
                new_len: nl,
            });
        }
    }
    hunks
}

/// Translates an old-side line to the new side, given hunks sorted by
/// `old_start`. Returns `(new_line, touched)` where `touched` is true if the
/// line fell inside a changed hunk (its content may differ, not just its
/// position).
fn translate_line(line: u32, hunks: &[Hunk]) -> (u32, bool) {
    let mut delta: i64 = 0;
    for h in hunks {
        let old_end = h.old_start + h.old_len; // exclusive
        if old_end <= line {
            // Hunk entirely before the line: shift by its size change.
            delta += i64::from(h.new_len) - i64::from(h.old_len);
        } else if line >= h.old_start && line < old_end {
            // Line is inside the changed region: best-effort to the new hunk
            // start (already an absolute new-side coordinate, so `delta` does
            // not apply), flagged as touched so the resolver lowers
            // confidence.
            return (h.new_start.max(1), true);
        } else {
            // Hunk is after the line; nothing more applies.
            break;
        }
    }
    let mapped = i64::from(line) + delta;
    (u32::try_from(mapped.max(1)).unwrap_or(1), false)
}

/// Translates a whole range, preserving a sane `start <= end`.
fn translate(range: LineRange, hunks: &[Hunk]) -> Transport {
    let (ns, ts) = translate_line(range.start(), hunks);
    let (ne, te) = translate_line(range.end(), hunks);
    let (lo, hi) = (ns.min(ne), ns.max(ne));
    match LineRange::new(lo, hi) {
        Ok(to) => Transport::Moved {
            to,
            touched: ts || te,
        },
        Err(_) => Transport::Unknown,
    }
}

/// Runs `git -C <dir> <args>` and returns trimmed stdout on success, or
/// `None` on any failure (no binary, non-zero exit, non-UTF-8 output). The
/// engine stays synchronous; this is a blocking child process.
fn run(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
