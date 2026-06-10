//! The `ynotes` binary.
//!
//! Deliberately thin. Its only jobs: parse arguments ([`cli`]), set up logging
//! ([`logging`]), dispatch to a subcommand ([`commands`]), and turn the result
//! into an exit code ([`command_error`]).
//!
//! One subtlety lives here rather than in a subcommand: an *argument* error is
//! caught by clap before any subcommand runs, so the `--json` failure envelope
//! the agent contract promises "regardless of exit code" has to be rendered
//! from [`exit_on_parse_error`] — otherwise a `--json` consumer would get empty
//! stdout plus a clap diagnostic on stderr for a bad invocation.
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
    // `try_parse`, not `parse`: `parse` lets clap print to stderr and exit the
    // process itself, which would bypass the `--json` envelope for every
    // argument-level error. Catching the error lets `exit_on_parse_error`
    // honour the contract.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => return exit_on_parse_error(&err),
    };
    logging::init(cli.verbose);

    match commands::dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // A `Rendered` error has already written a JSON envelope to
            // stdout; printing the unstructured message to stderr too would
            // give a `--json` consumer two streams to reconcile. Every other
            // error class gets the usual one-line diagnostic.
            if !matches!(err, command_error::CommandError::Rendered { .. }) {
                eprintln!("{PROGRAM}: {err}");
            }
            // The full chain goes to debug-level tracing so `-vv` (or
            // YNOTES_LOG) reveals the cause without cluttering normal output.
            tracing::debug!(error = ?err, "command failed");
            err.exit_code()
        }
    }
}

/// Turn a clap argument-parsing failure into the right exit, preserving clap's
/// native human output except where the `--json` agent contract requires the
/// failure envelope on stdout instead.
///
/// Three cases:
/// 1. `--help` / `--version` — successful outputs, not errors. clap prints them
///    to stdout and exits `0`, even when `--json` is also present: a consumer
///    asking for help wants the help text, not a machine payload.
/// 2. A usage error *without* `--json` — clap's native behaviour: the
///    diagnostic on stderr, stdout empty, exit `2`.
/// 3. A usage error *with* `--json` — the agent contract promises the envelope
///    on stdout regardless of exit code, so render the failure envelope
///    (`type: "usage"`) there and exit `2`. clap's rendered message is exactly
///    the text that would otherwise have gone to stderr, which is what the
///    schema's `error` field is defined to carry.
fn exit_on_parse_error(err: &clap::Error) -> ExitCode {
    use clap::error::ErrorKind;

    let is_help_or_version = matches!(
        err.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    if is_help_or_version || !wants_json() {
        // `err.exit()` writes to the correct stream and exits with the correct
        // code (`0` for help/version, `2` for a usage error) — the exact native
        // behaviour, untouched.
        err.exit();
    }

    commands::json_envelope::print_error(
        err.to_string().trim(),
        commands::json_envelope::ErrorKind::Usage,
    );
    ExitCode::from(command_error::codes::USAGE)
}

/// Whether `--json` was requested, by scanning the raw arguments.
///
/// On a *parse failure* there is no parsed [`Cli`] to read the flag from, and
/// `--json` is a per-subcommand flag rather than a global, so the raw scan is
/// the only signal available this early. Arguments after a `--` separator are
/// positional values, not flags, so the scan stops there — `ynotes -- --json`
/// is not a request for JSON output.
fn wants_json() -> bool {
    std::env::args()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--json")
}
