//! `tmx env` — drive a Flow's environment provider through its lifecycle methods (07 §`tmx env`).
//!
//! Maps a provider method 1:1 (`bootstrap`/`deploy`/`clean`/`destroy`) plus the `up`/`down`
//! aggregates onto the [`EnvironmentProvider`](tmx_core::ports::driven::EnvironmentProvider) port
//! (06 §Environment and provider execution). It resolves the Flow (the same search order as
//! `tmx run`), preflights it, loads and options-checks the provider named by its
//! `environment.provider`, then invokes the selected method(s), returning a JSON summary the binary
//! prints to stdout. A failed method is an `environment` error (exit 5), distinct from a pipeline
//! `RunFailure` (exit 1).
//!
//! **Aggregate teardown is best-effort.** `up` (`bootstrap` → `deploy`) stops on the first failure;
//! `down` (`clean` → `destroy`) attempts *both* even when the first fails, then reports the first
//! error — so a partial teardown still runs everything it can, mirroring the run wrapper's
//! best-effort `clean`.

use serde_json::{Value, json};

use tmx_core::ports::driven::{ProviderMethod, ProviderOutcome};
use tmx_core::{RunError, preflight};
use tmx_schema::Environment;

use crate::args::{EnvMethod, RunArgs};
use crate::commands::lifecycle::{invoke_method, load_provider};
use crate::commands::run::resolve_target;
use crate::compose::Composed;
use crate::config;

/// Run `tmx env <method> [flow]`, returning a JSON summary of the method(s) invoked.
///
/// # Errors
///
/// Returns a typed [`RunError`]: `resolution` for an unresolved Flow/manifest, `validation` for a
/// malformed artifact or an out-of-schema options block, or `environment` (exit 5) for a failed
/// provider method or a Flow that declares no provider.
pub async fn execute(args: crate::args::EnvArgs) -> Result<Value, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    // Reuse `tmx run`'s Flow resolution: `tmx env` targets the same Flow surface.
    let run_args = RunArgs {
        flow: args.flow.clone(),
        file: args.file.clone(),
        ..RunArgs::default()
    };
    let resolved = resolve_target(&run_args, &cwd, config::env_flow())?;

    // `tmx env` drives the provider lifecycle, not the pipeline, so no run events stream; the reporter
    // is unused. Compose it with the machine-data default (json, no colour).
    let composed = Composed::new(
        resolved.base_dir.clone(),
        tmx_adapters::sink::Format::Json,
        false,
    )?;
    let preflighted = preflight(
        &resolved.target,
        composed.preflight_ports(),
        &composed.available_capabilities(),
    )
    .await?;
    for warning in &preflighted.warnings {
        eprintln!("warning: {}", warning.message);
    }

    let environment = preflighted.flow.environment.clone().ok_or_else(|| {
        RunError::new(
            tmx_core::ErrorCategory::Environment,
            "no_environment",
            "`tmx env` needs a Flow that declares an `environment` block with a provider",
        )
    })?;

    let loaded = load_provider(&environment, &composed).await?;

    let (methods, best_effort) = plan(args.method);
    let outcomes = run_plan(&loaded, &composed, &environment, &methods, best_effort).await?;

    Ok(json!({
        "provider": loaded.manifest.name,
        "method": method_label(args.method),
        "results": outcomes,
    }))
}

/// The ordered lifecycle methods a selector expands to, and whether teardown is best-effort.
///
/// A single method maps 1:1; `up` is `bootstrap` → `deploy` (stop on first failure); `down` is
/// `clean` → `destroy` (best-effort: attempt both). `clean`/`destroy` on their own are teardown, so
/// they are best-effort too — a single teardown call still returns its error, but the flag documents
/// intent and keeps the aggregate/single paths symmetric.
fn plan(method: EnvMethod) -> (Vec<ProviderMethod>, bool) {
    match method {
        EnvMethod::Bootstrap => (vec![ProviderMethod::Bootstrap], false),
        EnvMethod::Deploy => (vec![ProviderMethod::Deploy], false),
        EnvMethod::Clean => (vec![ProviderMethod::Clean], true),
        EnvMethod::Destroy => (vec![ProviderMethod::Destroy], true),
        EnvMethod::Up => (
            vec![ProviderMethod::Bootstrap, ProviderMethod::Deploy],
            false,
        ),
        EnvMethod::Down => (vec![ProviderMethod::Clean, ProviderMethod::Destroy], true),
    }
}

/// The stable label for a selector, echoed in the JSON summary.
fn method_label(method: EnvMethod) -> &'static str {
    match method {
        EnvMethod::Bootstrap => "bootstrap",
        EnvMethod::Deploy => "deploy",
        EnvMethod::Clean => "clean",
        EnvMethod::Destroy => "destroy",
        EnvMethod::Up => "up",
        EnvMethod::Down => "down",
    }
}

/// Invoke each planned method, collecting its JSON outcome.
///
/// When `best_effort` is false, the first failure short-circuits and is returned (later methods do
/// not run). When it is true, every method is attempted regardless, and the *first* error seen is
/// returned after the whole plan has run — so a `down` aggregate tears down everything it can before
/// reporting.
async fn run_plan(
    loaded: &tmx_adapters::provider::LoadedProviderManifest,
    composed: &Composed,
    environment: &Environment,
    methods: &[ProviderMethod],
    best_effort: bool,
) -> Result<Vec<Value>, RunError> {
    let mut outcomes = Vec::with_capacity(methods.len());
    let mut first_error: Option<RunError> = None;
    for &method in methods {
        match invoke_method(loaded, composed, environment, method).await {
            Ok(outcome) => outcomes.push(outcome_json(&outcome)),
            Err(error) => {
                if !best_effort {
                    return Err(error);
                }
                // Best-effort: record the first error, keep tearing down the rest of the plan.
                eprintln!(
                    "tmx: warning: provider method `{}` failed: {}",
                    method.as_str(),
                    error.message
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(outcomes),
    }
}

/// Render one [`ProviderOutcome`] as JSON for the summary.
fn outcome_json(outcome: &ProviderOutcome) -> Value {
    json!({
        "method": outcome.method.as_str(),
        "ms": outcome.ms.0,
        "output": outcome.output.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_expands_singles_and_aggregates() {
        // A single method maps 1:1 and is not best-effort (except teardown); up/down expand in order.
        assert_eq!(
            plan(EnvMethod::Deploy),
            (vec![ProviderMethod::Deploy], false)
        );
        assert_eq!(
            plan(EnvMethod::Up),
            (
                vec![ProviderMethod::Bootstrap, ProviderMethod::Deploy],
                false
            ),
            "up is bootstrap then deploy, stop-on-first-failure"
        );
        assert_eq!(
            plan(EnvMethod::Down),
            (vec![ProviderMethod::Clean, ProviderMethod::Destroy], true),
            "down is clean then destroy, best-effort teardown"
        );
        // Teardown singles are best-effort; provisioning singles are not.
        assert!(plan(EnvMethod::Clean).1, "clean is best-effort");
        assert!(!plan(EnvMethod::Bootstrap).1, "bootstrap stops on failure");
    }

    #[test]
    fn method_label_names_every_selector() {
        // Every selector has a distinct label — the summary key that names what ran.
        let labels = [
            EnvMethod::Bootstrap,
            EnvMethod::Deploy,
            EnvMethod::Clean,
            EnvMethod::Destroy,
            EnvMethod::Up,
            EnvMethod::Down,
        ]
        .map(method_label);
        let mut distinct = labels.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            labels.len(),
            "every selector label is distinct"
        );
        assert_eq!(
            method_label(EnvMethod::Up),
            "up",
            "the up aggregate is labelled up"
        );
    }
}
