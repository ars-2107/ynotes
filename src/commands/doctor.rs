//! `ynotes doctor` — report the environment ynotes resolves against.
//!
//! Strictly read-only: it mutates nothing (invariant 8 — a resolve never
//! writes, and `doctor` writes nothing it resolves). Two jobs: make a bug
//! report self-contained (version, platform), and surface a *degraded* state
//! before it puzzles a user — chiefly a missing `git` binary, which silently
//! disables R1's transport rung.
//!
//! `--json` emits the same facts as a single object so an agent can read the
//! environment without parsing the human text.

use std::io::Write;

use serde::Serialize;

use super::json_envelope;
use crate::command_error::CommandError;

/// The discovered store, for the JSON report: a found-and-readable store
/// carries its path and note count; a missing or unreadable one carries a
/// reason instead, mirroring the three states the human form distinguishes.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum StoreReport {
    /// A store was found and its notes could be counted.
    Found {
        /// The `.ynotes` directory backing the store.
        path: String,
        /// How many notes it holds.
        notes: usize,
    },
    /// No store was found in any ancestor of the current directory.
    None,
    /// A store was found but could not be read; `reason` is the engine error.
    Unreadable {
        /// The `.ynotes` directory, when one was located.
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Why the store (or its notes) could not be read.
        reason: String,
    },
}

/// The machine-readable environment report (`doctor --json`).
#[derive(Serialize)]
struct DoctorReport<'a> {
    /// The compiled ynotes version.
    version: &'a str,
    /// `OS-ARCH`, the same string the human form prints.
    platform: String,
    /// The `git` executable's version, or `null` when no `git` binary is on
    /// `PATH` (R1's transport rung is then disabled).
    git: Option<String>,
    /// The store the current directory resolves to.
    store: StoreReport,
}

/// Write diagnostic information to stdout — human text, or JSON when `json`.
///
/// # Errors
///
/// Returns [`CommandError::Io`] if writing to stdout fails — for example when
/// the output is piped into a process that exits early (broken pipe) — or
/// [`CommandError::Render`] if the JSON report cannot be serialised.
pub(crate) fn run(json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(run_json)
    } else {
        run_text()
    }
}

/// Emit the environment report as a single JSON object, wrapped in the
/// shared `{success, v, data}` envelope.
fn run_json() -> Result<(), CommandError> {
    let report = DoctorReport {
        version: ynotes::version(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        git: ynotes::git_version(),
        store: match ynotes::Store::discover() {
            Ok(store) => {
                let path = store.root().display().to_string();
                match store.all_notes() {
                    Ok(notes) => StoreReport::Found {
                        path,
                        notes: notes.len(),
                    },
                    Err(e) => StoreReport::Unreadable {
                        path: Some(path),
                        reason: e.to_string(),
                    },
                }
            }
            Err(ynotes::Error::StoreNotFound { .. }) => StoreReport::None,
            Err(e) => StoreReport::Unreadable {
                path: None,
                reason: e.to_string(),
            },
        },
    };
    json_envelope::print_success(&report)
}

/// Emit the environment report as the human text format.
fn run_text() -> Result<(), CommandError> {
    // Lock stdout once and write through the guard: fewer syscalls, and no
    // interleaving if this ever logs concurrently.
    let mut out = std::io::stdout().lock();

    writeln!(out, "{:<10}{}", "ynotes", ynotes::version())?;
    writeln!(
        out,
        "{:<10}{}-{}",
        "platform",
        std::env::consts::OS,
        std::env::consts::ARCH
    )?;

    // git is an optional runtime dependency: R1's transport rung shells out to
    // it and is skipped when absent. Report the degradation here rather than
    // leaving a user to infer it from weaker anchors.
    match ynotes::git_version() {
        Some(version) => writeln!(out, "{:<10}{version}", "git")?,
        None => writeln!(
            out,
            "{:<10}not found (R1 git transport disabled; other rungs unaffected)",
            "git"
        )?,
    }

    // The store is per-directory; doctor reports whichever one the current
    // directory resolves to, exactly as a real subcommand would. A missing
    // store is a normal state (point the user at `init`); a store that exists
    // but cannot be read is *not* — report it as the fault it is rather than
    // collapsing it into "none found".
    match ynotes::Store::discover() {
        Ok(store) => {
            let root = store.root().display();
            match store.all_notes() {
                Ok(notes) => {
                    let n = notes.len();
                    let plural = if n == 1 { "" } else { "s" };
                    writeln!(out, "{:<10}{root} ({n} note{plural})", "store")?;
                }
                Err(e) => writeln!(out, "{:<10}{root} (notes unreadable: {e})", "store")?,
            }
        }
        Err(ynotes::Error::StoreNotFound { .. }) => {
            writeln!(out, "{:<10}none found (run `ynotes init`)", "store")?;
        }
        Err(e) => writeln!(out, "{:<10}unreadable: {e}", "store")?,
    }

    Ok(())
}
