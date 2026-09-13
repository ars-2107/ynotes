//! Minimal, opt-out terminal colour for the human text renderers.
//!
//! Binary-only (presentation, never the engine). No hardcoded ANSI leaks
//! elsewhere; this is the single place that emits escape codes, and only when
//! it is appropriate to: colour is suppressed when `NO_COLOR` is set (any
//! value, per <https://no-color.org>) or stdout is not a terminal, so piped
//! and machine-read output (including `--json`) is always plain.

use std::io::IsTerminal as _;
use std::sync::OnceLock;

/// A foreground colour for a status word.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Colour {
    /// Anchored, trustworthy.
    Green,
    /// Drifted, found but changed/moved.
    Yellow,
    /// Orphaned, lost.
    Red,
    /// An actionable advisory, e.g. a note surfaced via a followed rename.
    Cyan,
    /// De-emphasised detail (confidence, rung lines).
    Dim,
}

impl Colour {
    /// The SGR parameter for this colour.
    fn code(self) -> &'static str {
        match self {
            Colour::Green => "32",
            Colour::Yellow => "33",
            Colour::Red => "31",
            Colour::Cyan => "36",
            Colour::Dim => "2",
        }
    }
}

/// Whether colour should be emitted at all (computed once).
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
}

/// `text` wrapped in `colour`, or `text` unchanged when colour is disabled.
pub(crate) fn paint(text: &str, colour: Colour) -> String {
    if enabled() {
        format!("\x1b[{}m{text}\x1b[0m", colour.code())
    } else {
        text.to_owned()
    }
}
