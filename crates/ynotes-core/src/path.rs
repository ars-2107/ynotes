//! The repo-relative, forward-slash path string that identifies a note.
//!
//! A note's `target` — and a git selector's `path` — is the file's location
//! relative to the work tree, always written with `/` separators whatever the
//! host OS. That keeps a `.ynotes` store portable and lets `save` and `query`
//! agree on the same key on every platform.
//!
//! Producing that string is **not** a blanket `\` → `/` replacement. On
//! Windows `\` is a path separator and must be converted; on Unix `\` is an
//! ordinary, legal filename character and must be left untouched.
//! [`to_forward_slash`] draws the line by walking
//! [`std::path::Path::components`] — the OS has already split the path on its
//! real separators, so a literal backslash inside a Unix filename survives.
//!
//! Two paths that name the same physical file must collapse to the same
//! target string before hashing into a note id (invariant #6). The job is
//! split: [`canonical_for_compare`] resolves symlinks and `..` segments via
//! the OS where it can, and [`lexical_normalise`] is the pure-lexical
//! fallback for paths whose deepest ancestor does not yet exist on disk.

use std::path::{Component, Path, PathBuf};

/// Joins the components of a relative `path` with `/`.
///
/// Each component is taken verbatim, so a character that is *data* rather than
/// a separator on the host OS (a backslash on Unix) is preserved, not
/// rewritten. `path` is expected to be relative — the output of
/// [`Path::strip_prefix`].
pub(crate) fn to_forward_slash(path: &Path) -> String {
    let mut out = String::new();
    for component in path.components() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&component.as_os_str().to_string_lossy());
    }
    out
}

/// Resolves `path` for an equality comparison: symlinks followed and `..`
/// segments collapsed, so two spellings of the same physical file produce
/// the same [`PathBuf`].
///
/// Strategy: walk up the chain looking for the deepest ancestor that exists,
/// canonicalise that with the OS (resolving symlinks), then re-attach the
/// non-existent tail with its `..` segments collapsed lexically. This works
/// for paths that do not exist yet (the `save` target is always real, but
/// `query`/`delete`/`list` accept paths whose target may have been removed
/// from disk while still indexed in the store).
///
/// Returns the input made absolute and lexically-normalised if no ancestor
/// can be canonicalised — the conservative fallback when the filesystem is
/// not consultable.
pub(crate) fn canonical_for_compare(path: &Path) -> PathBuf {
    let Ok(absolute) = std::path::absolute(path) else {
        return lexical_normalise(path);
    };

    let mut current: PathBuf = absolute.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canon) = current.canonicalize() {
            let mut out = canon;
            for piece in tail.iter().rev() {
                out.push(piece);
            }
            return lexical_normalise(&out);
        }
        let Some(name) = current.file_name().map(std::ffi::OsString::from) else {
            return lexical_normalise(&absolute);
        };
        tail.push(name);
        if !current.pop() {
            return lexical_normalise(&absolute);
        }
    }
}

/// Lexically collapses `.` and `..` components without touching the
/// filesystem. Used both as a building block of [`canonical_for_compare`]
/// and as the standalone fallback for paths whose ancestors cannot be
/// canonicalised.
///
/// A leading `..` (i.e. an attempt to escape an absolute root or a relative
/// path that cannot pop further) is preserved verbatim — collapsing it would
/// silently rewrite the path's meaning.
pub(crate) fn lexical_normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out.components().next_back();
                match popped {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    // Nothing to pop, or popping a root/prefix would change
                    // meaning — keep the `..` literally.
                    _ => out.push(".."),
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{lexical_normalise, to_forward_slash};

    #[test]
    fn joins_components_with_forward_slashes() {
        assert_eq!(to_forward_slash(Path::new("src/note.rs")), "src/note.rs");
        assert_eq!(to_forward_slash(Path::new("a")), "a");
    }

    /// On Unix a backslash is an ordinary filename character — data, not a
    /// separator — so it must survive intact rather than be rewritten to `/`.
    #[cfg(unix)]
    #[test]
    fn a_backslash_in_a_unix_filename_is_preserved() {
        assert_eq!(to_forward_slash(Path::new("we\\ird.rs")), "we\\ird.rs");
    }

    #[test]
    fn lexical_normalise_collapses_dot_and_parent_dir() {
        assert_eq!(lexical_normalise(Path::new("a/./b")), PathBuf::from("a/b"));
        assert_eq!(
            lexical_normalise(Path::new("a/b/../c")),
            PathBuf::from("a/c")
        );
        assert_eq!(
            lexical_normalise(Path::new("a/b/../../c")),
            PathBuf::from("c")
        );
    }

    /// A `..` that cannot be popped lexically (no preceding `Normal`
    /// component) is preserved — collapsing it silently would change the
    /// path's meaning. Tested on absolute and bare-relative forms.
    #[test]
    fn lexical_normalise_keeps_a_dangling_parent_dir() {
        assert_eq!(lexical_normalise(Path::new("..")), PathBuf::from(".."));
        assert_eq!(lexical_normalise(Path::new("../a")), PathBuf::from("../a"));
    }
}
