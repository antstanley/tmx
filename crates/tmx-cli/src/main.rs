#![forbid(unsafe_code)]
//! `tmx` — the command-line binary and composition root.
//!
//! The single driving adapter for the runtime (02 §Composition root, 07 §Implementation layout). Its
//! job: parse arguments (`clap`, [`args`]), compose the concrete `tmx-adapters` implementations into
//! the `tmx-core` use cases ([`compose`]), dispatch the requested command ([`commands`]), render the
//! result, and — as the *only* place this mapping lives — translate a core [`ErrorCategory`] into a
//! process exit code ([`exit_code`]). No business logic lives here; it parses, composes, calls a use
//! case, serialises the masked final state to stdout, and maps the outcome to an exit code.
//!
//! ## stdout / stderr contract (07 §stdout / stderr contract)
//!
//! **stdout** carries machine data only: the final Pipeline state as one JSON object (secrets masked),
//! so `tmx run flow.yaml | jq` works with no flag. **stderr** carries human progress (per-task lines
//! from the reporter) and any error. Nothing else is ever written to stdout.
//!
//! Depends on `tmx-adapters`, `tmx-core`, and `tmx-schema`.

mod args;
mod commands;
mod compose;
mod config;

use clap::Parser;
use serde_json::Value;

use tmx_core::{ErrorCategory, RunError, RunRecord, RunStatus};

use crate::args::{Cli, Command};

/// Exit code for a run that *completed* — a usage error clap handles itself; a core failure is mapped
/// by [`exit_code`]. `0` is success; a completed-but-failed run is mapped by [`exit_for_status`].
const EXIT_SUCCESS: i32 = 0;

fn main() {
    let cli = Cli::parse();
    // The process adapter awaits real tokio I/O, so the use cases run on a Tokio runtime built here.
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("tmx: could not start the async runtime: {error}");
            std::process::exit(exit_code(&RunError::new(
                ErrorCategory::Environment,
                "runtime_unavailable",
                "the async runtime failed to start",
            )));
        }
    };

    let code = match cli.command {
        Command::Run(run_args) => match runtime.block_on(commands::run::execute(run_args)) {
            Ok(record) => {
                // Machine data on stdout: the masked final Pipeline state as one JSON object.
                print_final_state(&record);
                exit_for_status(record.status)
            }
            Err(error) => {
                // Human-facing error on stderr; stdout stays empty so a `| jq` pipeline sees no data.
                eprintln!("tmx: {error}");
                exit_code(&error)
            }
        },
    };
    std::process::exit(code);
}

/// Write the masked final Pipeline state as one JSON object to **stdout** — the sole stdout output, so
/// `tmx run flow.yaml | jq` parses it with no flag. A run that captured no state prints `{}`.
fn print_final_state(record: &RunRecord) {
    let state = record.final_state.as_ref().map_or_else(
        || Value::Object(serde_json::Map::new()),
        |s| s.as_value().clone(),
    );
    match serde_json::to_string_pretty(&state) {
        Ok(text) => println!("{text}"),
        // A state that fails to serialise is unexpected (it is a JSON object by construction); emit an
        // empty object so stdout still carries valid JSON rather than nothing or a partial fragment.
        Err(_) => println!("{{}}"),
    }
}

/// Map a core [`RunError`]'s [`ErrorCategory`] to its documented process exit code (07 §Exit codes).
///
/// This is the **only** place in the codebase that names a process exit code: the core returns
/// categories, the driving adapter maps them (an HTTP host would map the same categories to status
/// codes instead). The `match` is exhaustive with no wildcard, so a new category cannot ship without a
/// deliberate code here.
#[must_use]
fn exit_code(error: &RunError) -> i32 {
    match error.category {
        ErrorCategory::RunFailure => 1,
        ErrorCategory::Validation => 3,
        ErrorCategory::Resolution => 4,
        ErrorCategory::Environment => 5,
        ErrorCategory::Timeout => 124,
        ErrorCategory::Interrupt => 130,
    }
}

/// Map a *completed* run's terminal [`RunStatus`] to an exit code. A run that finishes with a failed
/// task returns `Ok(record)` with a `failed` status — not an `Err` — so the failure is mapped here to
/// the same code its `run_failure` category would yield (07 §Exit codes). The `match` is exhaustive.
#[must_use]
fn exit_for_status(status: RunStatus) -> i32 {
    match status {
        RunStatus::Ok => EXIT_SUCCESS,
        RunStatus::Failed => 1,
        RunStatus::TimedOut => 124,
        RunStatus::Cancelled => 130,
        // A returned record is always terminal; a non-terminal status here is a defensive fallback
        // treated as a run failure rather than a spurious success.
        RunStatus::Pending | RunStatus::Running => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_maps_every_category_to_its_documented_code() {
        // The full 07 §Exit codes table: each category maps to exactly its documented code.
        assert_eq!(
            exit_code(&RunError::run_failure("x", "m")),
            1,
            "run_failure → 1"
        );
        assert_eq!(
            exit_code(&RunError::validation("x", "m")),
            3,
            "validation → 3"
        );
        assert_eq!(
            exit_code(&RunError::resolution("x", "m")),
            4,
            "resolution → 4"
        );
        assert_eq!(
            exit_code(&RunError::new(ErrorCategory::Environment, "x", "m")),
            5,
            "environment → 5"
        );
        assert_eq!(
            exit_code(&RunError::new(ErrorCategory::Timeout, "x", "m")),
            124,
            "timeout → 124"
        );
        assert_eq!(
            exit_code(&RunError::new(ErrorCategory::Interrupt, "x", "m")),
            130,
            "interrupt → 130"
        );
        // Every category has a distinct, documented code — no two collide.
        let mut codes: Vec<i32> = ErrorCategory::ALL
            .iter()
            .map(|c| exit_code(&RunError::new(*c, "x", "m")))
            .collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ErrorCategory::ALL.len(), "codes are distinct");
    }

    #[test]
    fn exit_for_status_maps_success_and_failure() {
        // Success is 0; a completed-but-failed run is 1 (matching run_failure); cancellation/timeout
        // carry their signal codes.
        assert_eq!(exit_for_status(RunStatus::Ok), 0, "ok → 0");
        assert_eq!(exit_for_status(RunStatus::Failed), 1, "failed → 1");
        assert_eq!(exit_for_status(RunStatus::TimedOut), 124, "timed_out → 124");
        assert_eq!(
            exit_for_status(RunStatus::Cancelled),
            130,
            "cancelled → 130"
        );
    }
}
