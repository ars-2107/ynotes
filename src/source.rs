//! The unit the anchoring engine reasons about: a file as a sequence of
//! lines, plus the line-range primitive used to address regions of it.
//!
//! The engine never touches the filesystem directly while resolving an
//! anchor — it operates on a [`SourceFile`] read once up front. Keeping that
//! boundary here means the selector rungs are pure functions of in-memory
//! text, which makes them trivially testable and free of I/O ordering
//! concerns. This module knows nothing of git or the on-disk store.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A one-based, inclusive range of lines, `start..=end` with `1 <= start <=
/// end`.
///
/// One-based and inclusive because that is the convention every editor and
/// `grep`/`git` user already has in their head; storing it in any other form
/// would mean translating at every boundary. The invariant (`start >= 1` and
/// `start <= end`) is enforced at construction so no rung has to re-check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    start: u32,
    end: u32,
}

impl LineRange {
    /// Builds a range, enforcing `1 <= start <= end`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if `start` is zero or `end < start`; the
    /// message names the offending values so a caller can surface them.
    pub fn new(start: u32, end: u32) -> Result<Self> {
        if start == 0 {
            return Err(Error::Invalid(format!(
                "line range start must be >= 1, got {start}"
            )));
        }
        if end < start {
            return Err(Error::Invalid(format!(
                "line range end {end} is before start {start}"
            )));
        }
        Ok(Self { start, end })
    }

    /// The first line in the range (one-based, inclusive).
    #[must_use]
    pub fn start(self) -> u32 {
        self.start
    }

    /// The last line in the range (one-based, inclusive).
    #[must_use]
    pub fn end(self) -> u32 {
        self.end
    }

    /// Whether `line` falls within this range.
    #[must_use]
    pub fn contains_line(self, line: u32) -> bool {
        self.start <= line && line <= self.end
    }

    /// Whether this range shares at least one line with `other`.
    #[must_use]
    pub fn overlaps(self, other: LineRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// A source file read into memory as its sequence of lines.
///
/// Lines are split with [`str::lines`], so a trailing newline does not yield a
/// spurious empty final line and `\r\n` is normalised to `\n` — both are what
/// line-addressed anchoring wants.
#[derive(Debug, Clone)]
pub struct SourceFile {
    path: PathBuf,
    lines: Vec<String>,
}

impl SourceFile {
    /// Reads `path` into memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be read, or [`Error::Invalid`]
    /// if its bytes are not valid UTF-8 (ynotes anchors text, not binaries).
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let text = String::from_utf8(bytes)
            .map_err(|_| Error::Invalid(format!("`{}` is not valid UTF-8", path.display())))?;
        Ok(Self {
            path: path.to_path_buf(),
            lines: text.lines().map(str::to_owned).collect(),
        })
    }

    /// Constructs a file view from already-decoded `lines` (used in tests and
    /// by callers that obtained the content from somewhere other than disk,
    /// e.g. a git blob).
    #[must_use]
    pub fn from_lines(path: PathBuf, lines: Vec<String>) -> Self {
        Self { path, lines }
    }

    /// The path this file was read from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's lines as a slice, for windowed matching.
    #[must_use]
    pub fn lines_slice(&self) -> &[String] {
        &self.lines
    }

    /// The whole file as one string (lines joined with `\n`), for a parser
    /// that wants the full text rather than line windows.
    #[must_use]
    pub fn text_all(&self) -> String {
        self.lines.join("\n")
    }

    /// The number of lines in the file.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        // A file cannot realistically exceed `u32::MAX` lines; the cast keeps
        // every line index in one integer type across the engine.
        u32::try_from(self.lines.len()).unwrap_or(u32::MAX)
    }

    /// The text of `range`, lines joined with `\n` (no trailing newline).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if `range` extends past the last line.
    pub fn text(&self, range: LineRange) -> Result<String> {
        let end = range.end() as usize;
        if end > self.lines.len() {
            return Err(Error::Invalid(format!(
                "range {}:{} extends past end of `{}` ({} lines)",
                range.start(),
                range.end(),
                self.path.display(),
                self.lines.len()
            )));
        }
        let start = (range.start() - 1) as usize;
        Ok(self.lines[start..end].join("\n"))
    }

    /// The `context`-line text immediately before and after `range`, clamped
    /// to the file bounds. Used to disambiguate an otherwise non-unique quote.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] if `range` extends past the last line.
    pub fn surrounding(&self, range: LineRange, context: u32) -> Result<(String, String)> {
        let end = range.end() as usize;
        if end > self.lines.len() {
            return Err(Error::Invalid(format!(
                "range {}:{} extends past end of `{}` ({} lines)",
                range.start(),
                range.end(),
                self.path.display(),
                self.lines.len()
            )));
        }
        let start = (range.start() - 1) as usize;
        let ctx = context as usize;
        let prefix = self.lines[start.saturating_sub(ctx)..start].join("\n");
        let suffix_end = (end + ctx).min(self.lines.len());
        let suffix = self.lines[end..suffix_end].join("\n");
        Ok((prefix, suffix))
    }
}
