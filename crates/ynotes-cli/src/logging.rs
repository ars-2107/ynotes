//! Process-wide `tracing` setup.
//!
//! Logs are diagnostics, not output: they go to **stderr** so stdout stays
//! clean for data a script or agent may parse.

use tracing_subscriber::EnvFilter;

/// Install the global tracing subscriber.
///
/// Level precedence:
///
/// 1. the `YNOTES_LOG` environment variable, if set (a full `EnvFilter`
///    directive such as `ynotes=debug`); otherwise
/// 2. the `-v` count: none → `warn`, `-v` → `info`, `-vv` → `debug`,
///    `-vvv`+ → `trace`.
///
/// Called once, at startup, before any subcommand runs.
pub(crate) fn init(verbosity: u8) {
    let fallback = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let filter = EnvFilter::try_from_env("YNOTES_LOG").unwrap_or_else(|_| EnvFilter::new(fallback));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}
