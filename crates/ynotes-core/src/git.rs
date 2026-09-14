//! Git subprocesses for range transport and rename discovery.
//!
//! Using the installed executable avoids a Git library dependency. Missing Git
//! or unavailable history disables that evidence; content matching can still
//! resolve notes. Validate revisions before passing them to subprocesses.

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

/// Whether `commit` is a full object id as `git rev-parse` emits one,
/// 40 lowercase hex digits (SHA-1) or 64 (SHA-256).
///
/// Every commit this module passes to git arrives from a note *record*, and
/// records are committed to the repository and shared with the team. A record
/// carrying `--output=…` instead of an object id would reach `git diff` as an
/// option rather than a revision, because `--` separates revisions from paths
/// and cannot protect the revision slot. Checking the shape here closes that
/// for every call site at once, and needs no `--end-of-options`, whose absence
/// on git older than 2.24 would silently disable R1 instead.
///
/// A rejected commit is not an error: each caller degrades to its documented
/// "git contributed nothing" answer and the remaining rungs resolve the note.
fn is_object_id(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64)
        && commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Git's rename set per baseline commit, fetched at most once each.
///
/// Rename detection costs a whole-tree diff and cannot be narrowed to one path
/// (a pathspec on the old name suppresses the pairing that finds the new one),
/// so the work is per *commit*, not per note. Without this memo a store with
/// `n` orphaned notes spawned `n` full-tree diffs on every `list` and every
/// `reanchor`, and notes overwhelmingly share a handful of baseline commits.
///
/// Scoped to one command run. The work tree can change under a long-lived
/// cache, and a read must answer about the tree as it is now.
#[derive(Debug, Default)]
pub(crate) struct RenameCache {
    by_commit: std::collections::HashMap<String, Vec<(String, String)>>,
}

impl RenameCache {
    /// Where `rel_path` was renamed to since `commit`, if anywhere.
    pub(crate) fn lookup(
        &mut self,
        ctx: &GitContext,
        commit: &str,
        rel_path: &str,
    ) -> Option<String> {
        self.by_commit
            .entry(commit.to_owned())
            .or_insert_with(|| ctx.renames_since(commit))
            .iter()
            .find(|(old, _)| old == rel_path)
            .map(|(_, new)| new.clone())
    }
}

impl GitContext {
    /// Discovers the repository containing `near`, or returns `None` if `near`
    /// is not in a git work tree or no `git` binary is available. Never errors
    ///, absence of git is a normal, non-fatal state for the ladder.
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
    /// Returns `Transport::Unknown` if git cannot diff that path, or if
    /// `from_commit` is not a well-formed object id (so R1 stays silent rather
    /// than guessing).
    #[must_use]
    pub fn transport(&self, from_commit: &str, rel_path: &str, range: LineRange) -> Transport {
        if !is_object_id(from_commit) {
            return Transport::Unknown;
        }
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

    /// Whether `rel_path` exists as a blob in `commit`'s tree, i.e. whether
    /// the file was tracked at that commit.
    ///
    /// `git cat-file -e <commit>:<rel_path>` exits zero iff that object
    /// exists, so the `run` helper yielding `Some` is exactly the "tracked"
    /// answer.
    /// Absence of the path, and any failure, including no `git` binary or a
    /// commit that is not a well-formed object id, answers `false`: an
    /// untracked file has no meaningful git baseline, so the caller must not
    /// attach a git selector for it.
    #[must_use]
    pub fn tracks(&self, commit: &str, rel_path: &str) -> bool {
        if !is_object_id(commit) {
            return false;
        }
        run(
            &self.root,
            &["cat-file", "-e", &format!("{commit}:{rel_path}")],
        )
        .is_some()
    }

    /// Count exact line-sequence occurrences in a file's saved Git baseline.
    ///
    /// Schema-v1 quote selectors did not persist capture-time uniqueness. A
    /// tracked note still carries `(commit, path)`, so the resolver can recover
    /// that missing identity evidence from the immutable blob instead of
    /// forcing every pre-v2 note to retain the older false-orphan behaviour.
    /// Best-effort: an unavailable object, git executable, non-UTF-8 blob, or
    /// malformed object expression contributes no evidence.
    #[must_use]
    pub(crate) fn exact_occurrences_at(
        &self,
        commit: &str,
        rel_path: &str,
        exact: &str,
    ) -> Option<u32> {
        if !is_object_id(commit) {
            return None;
        }
        let object = format!("{commit}:{rel_path}");
        let text = run(
            &self.root,
            &["show", "--no-ext-diff", "--no-textconv", &object],
        )?;
        let lines: Vec<&str> = text.lines().collect();
        let needle: Vec<&str> = exact.split('\n').collect();
        if needle.is_empty() || needle.len() > lines.len() {
            return Some(0);
        }
        let count = lines
            .windows(needle.len())
            .filter(|window| window.iter().zip(&needle).all(|(line, part)| line == part))
            .count();
        Some(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Expresses a working-tree line `range` in the coordinates of `commit`'s
    /// version of `rel_path`, the inverse of [`transport`](Self::transport).
    ///
    /// `save` and `reanchor` capture a region by its *working-tree* range,
    /// while the git rung's baseline is a *commit*. When the working tree is
    /// dirty those coordinate systems differ, so the captured range is
    /// translated back into the commit before it is stored, otherwise every
    /// later transport carries the dirty-diff skew. Best-effort: if git
    /// cannot diff that path the `range` is returned unchanged, and the git
    /// rung simply degrades like any other absent signal. A commit that is not
    /// a well-formed object id degrades the same way.
    #[must_use]
    pub fn range_in_commit(&self, commit: &str, rel_path: &str, range: LineRange) -> LineRange {
        if !is_object_id(commit) {
            return range;
        }
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
    /// work tree, if git detects it was renamed since, `None` when it was not
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
    /// Best-effort: any git failure, or a commit that is not a well-formed
    /// object id, yields `None`, so a missing tool never turns into an error.
    #[must_use]
    pub fn renamed_to(&self, from_commit: &str, rel_path: &str) -> Option<String> {
        self.renames_since(from_commit)
            .into_iter()
            .find(|(old, _)| old == rel_path)
            .map(|(_, new)| new)
    }

    /// Every `(old_path, new_path)` rename git detects between `from_commit`
    /// and the work tree, with `old_path` as it was in `from_commit` and
    /// `new_path` as it is now.
    ///
    /// Renames are taken one commit at a time (`git log --topo-order
    /// from_commit..HEAD`, oldest first), then the uncommitted diff against
    /// `HEAD`, and chained so a multi-hop move (`a` → `b` → `c`) reports
    /// `(a, c)`. A single whole-span diff cannot do this: rename similarity
    /// is measured against the baseline blob, so a file that was edited
    /// heavily *between* the baseline and its rename falls under the
    /// threshold and reads as delete plus add, even though the rename commit
    /// itself is a clean move.
    ///
    /// The chain composes only if steps arrive in ancestry order, which
    /// `--topo-order` guarantees; plain `git log --reverse` orders by commit
    /// date, and merged side branches can interleave out of ancestry order.
    /// Every commit is visited, including those on merged side branches, so
    /// each rename is compared against its own parent. `--first-parent` would
    /// not do: it diffs a merge against the first parent, which is the
    /// baseline-versus-rewritten comparison that fails the threshold.
    ///
    /// The whole-span diff is always unioned in last, for any origin the
    /// walk did not claim: a baseline that is not an ancestor of `HEAD` (a
    /// note saved on another branch) or history that is unavailable locally.
    /// An origin the walk did claim keeps the walk's answer even when the
    /// whole-span diff would pair it differently; content corroboration at
    /// the proposed path decides either way (invariant #10).
    ///
    /// The unit of work is the *commit*, not the path: git has to diff the
    /// whole tree either way, because scoping the pathspec to one side of a
    /// rename suppresses the pairing that detects it. Callers resolving many
    /// notes should therefore fetch this once per distinct baseline commit and
    /// look up each path in the result, rather than calling
    /// [`renamed_to`](Self::renamed_to) per note, otherwise a store with `n`
    /// orphans costs `n` history walks on every read.
    ///
    /// Empty on any git failure, or for a commit that is not a well-formed
    /// object id.
    #[must_use]
    pub fn renames_since(&self, from_commit: &str) -> Vec<(String, String)> {
        if !is_object_id(from_commit) {
            return Vec::new();
        }
        let mut steps = Vec::new();
        // Committed history, one record per rename step, oldest first in
        // ancestry order so the chain composes in the order the moves
        // happened. See the doc comment for why date order is not enough.
        let range = format!("{from_commit}..HEAD");
        if let Some(out) = run_history_prefix(
            &self.root,
            &[
                "log",
                "--reverse",
                "--topo-order",
                RENAME_FIND,
                "--name-status",
                "-z",
                "--diff-filter=R",
                "--format=",
                &range,
            ],
        ) {
            steps.extend(parse_renames_z(&out));
        }
        // Staged or working-tree rename, not in any commit yet, applied last.
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
            steps.extend(parse_renames_z(&out));
        }
        let mut chained = chain_renames(steps);
        // Whole-span union, for origins the walk could not see. An origin the
        // walk already claimed is not overridden.
        if let Some(out) = run(
            &self.root,
            &[
                "diff",
                RENAME_FIND,
                "--name-status",
                "-z",
                "--diff-filter=R",
                from_commit,
            ],
        ) {
            for (old, new) in parse_renames_z(&out) {
                if !chained.iter().any(|(seen, _)| *seen == old) {
                    chained.push((old, new));
                }
            }
        }
        chained
    }

    /// Every earlier path the work-tree file `rel_path` was renamed from, most
    /// recent first and de-duplicated, the reverse of
    /// [`renamed_to`](Self::renamed_to).
    ///
    /// Two sources are unioned so the reverse view is as complete as the
    /// forward one:
    ///
    /// - the **uncommitted** diff (`HEAD` against the work tree), so a
    ///   staged-but-not-yet-committed `git mv` is seen, git *history* cannot
    ///   show it because it is not in a commit yet;
    /// - the **committed history** via `git log --follow`, git's own
    ///   rename-following machinery, so a multi-hop chain (`a` → `b` →
    ///   `rel_path`) yields every prior name.
    ///
    /// Empty when the file has no rename history, is untracked, or git is
    /// unavailable, never an error.
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
        if let Some(out) = run_history_prefix(
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

/// Composes rename steps, given in the order they happened, into
/// `(origin, current)` pairs. A step whose old side is some chain's current
/// name extends that chain; anything else starts a new one. A chain that ends
/// back at its origin is dropped: the path was not renamed after all.
fn chain_renames(steps: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut chains: Vec<(String, String)> = Vec::new();
    for (old, new) in steps {
        if let Some(chain) = chains.iter_mut().find(|(_, current)| *current == old) {
            chain.1 = new;
        } else {
            chains.push((old, new));
        }
    }
    chains.retain(|(origin, current)| origin != current);
    chains
}

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
/// [`Skipped`](crate::anchor::RungResult::Skipped) when it is absent, by
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

/// Runs a git history query and keeps a valid stdout prefix even when deeper
/// traversal fails.
///
/// Partial clones can hold the recent commit that renamed a file while older
/// promised objects remain remote. Offline, `git log --follow` emits the local
/// rename records first and then exits non-zero when it reaches the missing
/// history. Those records are still authoritative local evidence. Discarding
/// them makes rename discovery silently weaker in either direction; the
/// forward walk additionally falls back to a whole-span diff against the
/// note's baseline.
///
/// This tolerance is deliberately isolated to committed rename history. Every
/// other git operation uses [`run`] and requires a successful exit status.
fn run_history_prefix(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::chain_renames;

    fn pairs(steps: &[(&str, &str)]) -> Vec<(String, String)> {
        steps
            .iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect()
    }

    #[test]
    fn chain_renames_composes_a_multi_hop_move() {
        assert_eq!(
            chain_renames(pairs(&[("a", "b"), ("b", "c")])),
            pairs(&[("a", "c")])
        );
    }

    #[test]
    fn chain_renames_keeps_independent_moves_apart() {
        assert_eq!(
            chain_renames(pairs(&[("a", "b"), ("x", "y")])),
            pairs(&[("a", "b"), ("x", "y")])
        );
    }

    #[test]
    fn chain_renames_drops_a_move_that_returned_home() {
        assert!(chain_renames(pairs(&[("a", "b"), ("b", "a")])).is_empty());
    }

    #[test]
    fn chain_renames_follows_a_name_reused_after_a_swap() {
        // a -> tmp, b -> a, tmp -> b: a and b swapped.
        assert_eq!(
            chain_renames(pairs(&[("a", "tmp"), ("b", "a"), ("tmp", "b")])),
            pairs(&[("a", "b"), ("b", "a")])
        );
    }
}
