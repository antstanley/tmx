//! `tmx lint` — the deeper static pass (resolution + dataflow) behind [`LintFlow`]
//! (07 §`tmx lint`; 03 §`lint`).
//!
//! Resolves the Flow by the same search order as `tmx run`, then runs the
//! [`EngineLintFlow`](tmx_core::EngineLintFlow) use case: it confirms `environment` / `context` /
//! `flow` references load, detects a cyclic `flow` import, checks `environment.options` against a
//! provider `optionsSchema`, and walks every `${{ tasks.NAME.field }}` / `inputs.*` / `secrets.*` read
//! against the Flow's declarations. Each finding is a warning [`Diagnostic`]; `main` prints them to
//! stderr and maps the outcome to an exit code — `0` when clean, `3` when a blocking finding (an error
//! diagnostic, or any warning under `--strict`) is present.

use tmx_adapters::sink::Format;

use tmx_core::EngineLintFlow;
use tmx_core::ports::driving::LintFlow;
use tmx_core::{Diagnostic, RunError, Severity};

use crate::args::{LintArgs, RunArgs};
use crate::commands::run::resolve_target;
use crate::compose::Composed;
use crate::config;

/// The result of a `tmx lint`: every finding, plus whether `--strict` was set (which decides whether a
/// warning is a blocking finding).
pub struct LintReport {
    /// The lint findings, in discovery order.
    pub diagnostics: Vec<Diagnostic>,
    /// Whether `--strict` promotes each warning to a blocking (exit-3) error.
    pub strict: bool,
}

impl LintReport {
    /// Whether the report is blocking (exit 3): any error-severity finding, or — under `--strict` —
    /// any finding at all. A clean report (or one carrying only warnings without `--strict`) is not.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        let has_error = self
            .diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Error));
        has_error || (self.strict && !self.diagnostics.is_empty())
    }
}

/// Run `tmx lint` to its [`LintReport`] (or a typed [`RunError`] for an unresolved Flow / unparseable
/// document).
///
/// # Errors
///
/// Returns a [`RunError`] when the Flow cannot be resolved (`resolution`) or does not parse as a Flow
/// (`validation`); a dataflow/resolution *finding* is data on the report, not an `Err`.
pub async fn execute(args: LintArgs) -> Result<LintReport, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    // Reuse `tmx run`'s Flow-resolution order (`--file` → positional → `$TMX_FLOW` → conventional file).
    let run_args = RunArgs {
        flow: args.flow.clone(),
        file: args.file.clone(),
        ..RunArgs::default()
    };
    let resolved = resolve_target(&run_args, &cwd, config::env_flow())?;
    let reference = resolved.file_reference.ok_or_else(|| {
        RunError::resolution(
            "lint_requires_file",
            "tmx lint operates on a single Flow file, not a directory layout",
        )
    })?;

    // Lint reaches no effecting port; the format/colour/store surface is irrelevant, so the composition
    // is built only for its resolve → load → validate ports.
    let composed = Composed::new(resolved.base_dir.clone(), Format::Json, false, None)?;
    let use_case = EngineLintFlow::new(composed.preflight_ports());
    let diagnostics = use_case.lint(&reference).await?;
    Ok(LintReport {
        diagnostics,
        strict: args.strict,
    })
}
