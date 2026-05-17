//! `ynotes doctor` — print the environment ynotes sees.
//!
//! Strictly read-only: it mutates nothing. Its purpose is to make bug reports
//! self-contained — a user or agent can paste the output and we can see the
//! version and platform without a round-trip.

use std::io::Write;

use crate::command_error::CommandError;

/// Write diagnostic information to stdout.
///
/// # Errors
///
/// Returns [`CommandError::Io`] if writing to stdout fails — for example when
/// the output is piped into a process that exits early (broken pipe).
pub(crate) fn run() -> Result<(), CommandError> {
    // Lock stdout once and write through the guard: fewer syscalls, and no
    // interleaving if this ever logs concurrently.
    let mut out = std::io::stdout().lock();
    writeln!(out, "ynotes   {}", ynotes::version())?;
    writeln!(out, "engine    ynotes {}", ynotes::version())?;
    writeln!(
        out,
        "platform  {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )?;
    Ok(())
}
