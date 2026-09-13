//! `ynotes confirm` presentation for explicit review-basis advancement.

use std::io::Write as _;

use ynotes::contract::confirm_view;
use ynotes::{Store, confirm};

use super::id::{resolve_unique, validate_prefix};
use super::json_envelope;
use super::render::short_id;
use crate::colour::{Colour, paint};
use crate::command_error::CommandError;

/// Confirm one note against its currently resolved code region.
///
/// # Errors
///
/// [`CommandError::Usage`] if the id is malformed, ambiguous, absent, or the
/// note is orphaned; engine and output failures otherwise. Under `--json`, a
/// failure is emitted as the standard agent-contract envelope.
pub(crate) fn run(id: &str, json: bool) -> Result<(), CommandError> {
    if json {
        json_envelope::wrap(|| run_inner(id, true))
    } else {
        run_inner(id, false)
    }
}

fn run_inner(id: &str, json: bool) -> Result<(), CommandError> {
    validate_prefix(id)?;
    let store = Store::discover()?;
    let original = resolve_unique(&store, id)?;
    let result = confirm(&store, original)?;

    if json {
        json_envelope::print_success(&confirm_view(&result))?;
    } else {
        let mut out = std::io::stdout().lock();
        if result.changed {
            writeln!(
                out,
                "{} {} (was {}) {} {}:{}",
                paint("confirmed", Colour::Green),
                short_id(&result.note.id),
                short_id(&result.previous_id),
                result.note.target,
                result.range.start(),
                result.range.end(),
            )?;
        } else {
            writeln!(
                out,
                "{} {} (review basis already current)",
                paint("unchanged", Colour::Dim),
                short_id(&result.note.id),
            )?;
        }
    }
    Ok(())
}
