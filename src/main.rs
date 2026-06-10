//! The `ynotes` binary.
//!
//! Deliberately thin. Its only jobs: parse arguments ([`cli`]), set up logging
//! ([`logging`]), dispatch to a subcommand ([`commands`]), and turn the result
//! into an exit code ([`command_error`]).
//!
//! These modules are compiled into the binary only — they are not part of the
//! `ynotes` library. All behaviour lives in the library (`use ynotes::…`).
//! If you add logic here that a non-CLI front-end would also need, it belongs
//! in the library instead. See AGENTS.md.

mod cli;
mod colour;
mod command_error;
mod commands;
mod logging;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;

/// Program name, used as the prefix on error output. Hard-coded so it cannot
/// disagree with the `[[bin]]` name in `Cargo.toml`.
const PROGRAM: &str = "ynotes";

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init(cli.verbose);

    match commands::dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // One terse line on stderr; the full chain goes to debug-level
            // tracing so `-vv` (or YNOTES_LOG) reveals the cause without
            // cluttering normal output.
            eprintln!("{PROGRAM}: {err}");
            tracing::debug!(error = ?err, "command failed");
            err.exit_code()
        }
    }
}
