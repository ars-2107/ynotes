//! `ynotes doctor` — report the environment ynotes resolves against.
//!
//! Strictly read-only: it mutates nothing (invariant 8 — a resolve never
//! writes, and `doctor` writes nothing it resolves). Three jobs: make a bug
//! report self-contained (version, platform), surface a *degraded* environment
//! before it puzzles a user — chiefly a missing `git` binary, which silently
//! disables R1's transport rung — and report *store health*: malformed note
//! records, index drift, and missing managed `.gitattributes` merge guards.
//! The health scan is lock-free and writes nothing (it must not perturb what it
//! diagnoses), so the read-only contract holds.
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

/// Index-drift counts for the health report (mirrors [`ynotes::IndexHealth`]).
#[derive(Serialize)]
struct IndexHealthView {
    /// Ids present on disk but missing from the index (a reindex restores).
    recovered: usize,
    /// Index pointers with no file behind them (a reindex drops).
    dangling: usize,
}

/// Store-health diagnostics for `doctor --json`. Defined locally (not by
/// deriving on the engine type) so the agent contract stays a deliberate
/// surface. Present only when a store was discovered.
#[derive(Serialize)]
struct HealthView {
    /// Paths under `.ynotes/notes/` that could not be read or parsed.
    malformed: Vec<String>,
    /// How far the index cache has drifted from the notes on disk.
    index: IndexHealthView,
    /// Managed `.gitattributes` lines absent from the store — the merge guards
    /// (`init`/`reindex` re-add them). Non-empty on a shared store means a
    /// future merge could corrupt it.
    gitattributes_missing: Vec<String>,
}

impl From<ynotes::StoreHealth> for HealthView {
    fn from(h: ynotes::StoreHealth) -> Self {
        Self {
            // Lossy is safe: these paths live under `.ynotes/notes/` and are
            // reported for inspection, not re-opened by key.
            malformed: h
                .malformed
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            index: IndexHealthView {
                recovered: h.index.recovered,
                dangling: h.index.dangling,
            },
            gitattributes_missing: h.gitattributes_missing,
        }
    }
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
    /// Store-health diagnostics. Present only when a store was discovered and
    /// its health could be read; absent when no store was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    health: Option<HealthView>,
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
    // Compute the store summary and its health together: both come from the
    // discovered store, and health runs even when `all_notes` fails (e.g. a
    // malformed index) — that is exactly what it diagnoses.
    let (store, health) = match ynotes::Store::discover() {
        Ok(store) => {
            let path = store.root().display().to_string();
            let store_report = match store.all_notes() {
                Ok(scan) => StoreReport::Found {
                    path: path.clone(),
                    notes: scan.notes.len(),
                },
                Err(e) => StoreReport::Unreadable {
                    path: Some(path.clone()),
                    reason: e.to_string(),
                },
            };
            // A health read that itself fails (a genuine I/O error) is omitted
            // rather than fatal — `doctor` still reports the rest.
            let health = store.health().ok().map(HealthView::from);
            (store_report, health)
        }
        Err(ynotes::Error::StoreNotFound { .. }) => (StoreReport::None, None),
        Err(e) => (
            StoreReport::Unreadable {
                path: None,
                reason: e.to_string(),
            },
            None,
        ),
    };
    let report = DoctorReport {
        version: ynotes::version(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        git: ynotes::git_version(),
        store,
        health,
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
            let root = store.root().display().to_string();
            match store.all_notes() {
                Ok(scan) => {
                    let n = scan.notes.len();
                    let plural = if n == 1 { "" } else { "s" };
                    writeln!(out, "{:<10}{root} ({n} note{plural})", "store")?;
                }
                Err(e) => writeln!(out, "{:<10}{root} (notes unreadable: {e})", "store")?,
            }
            // Health runs even when the note count above failed (a malformed
            // index): it is the diagnostic for exactly that. A health read that
            // itself errors is reported, not fatal.
            match store.health() {
                Ok(health) => write_health_text(&mut out, &health)?,
                Err(e) => writeln!(out, "{:<10}unreadable: {e}", "health")?,
            }
        }
        Err(ynotes::Error::StoreNotFound { .. }) => {
            writeln!(out, "{:<10}none found (run `ynotes init`)", "store")?;
        }
        Err(e) => writeln!(out, "{:<10}unreadable: {e}", "store")?,
    }

    Ok(())
}

/// Render the store-health block: a `health` summary line plus a detail line
/// per problem. Prints `ok` when nothing is wrong, so a clean store still shows
/// the health row (its absence would be ambiguous).
fn write_health_text(
    out: &mut impl Write,
    health: &ynotes::StoreHealth,
) -> Result<(), CommandError> {
    let malformed = health.malformed.len();
    let recovered = health.index.recovered;
    let dangling = health.index.dangling;
    let guards = health.gitattributes_missing.len();

    if malformed == 0 && recovered == 0 && dangling == 0 && guards == 0 {
        writeln!(out, "{:<10}ok", "health")?;
        return Ok(());
    }

    writeln!(
        out,
        "{:<10}{malformed} unreadable, index drift ({recovered} recovered, {dangling} dangling), {guards} missing merge guard{}",
        "health",
        if guards == 1 { "" } else { "s" },
    )?;
    for p in &health.malformed {
        writeln!(out, "  unreadable {}", p.display())?;
    }
    if malformed > 0 {
        // The path above ends in the record's id; `delete` takes the full id.
        writeln!(
            out,
            "  run `ynotes delete <id>` to remove an unreadable record"
        )?;
    }
    for line in &health.gitattributes_missing {
        writeln!(out, "  missing gitattributes rule: {line}")?;
    }
    if recovered > 0 || dangling > 0 {
        writeln!(
            out,
            "  run `ynotes reindex` to reconcile the index with notes/"
        )?;
    }
    Ok(())
}
