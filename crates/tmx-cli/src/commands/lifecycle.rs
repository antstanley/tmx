//! The ephemeral-environment lifecycle — the shared provider machinery `tmx run` and `tmx env` both
//! drive (06 §Environment and provider execution, §Ephemeral lifecycle).
//!
//! A Flow's `environment.provider` names an [`EnvironmentProvider`](tmx_core::ports::driven::EnvironmentProvider)
//! manifest (resolved as a filesystem reference in v0). [`load_provider`] loads and schema-validates
//! it and checks the Flow's `environment.options` against the manifest's `optionsSchema` — the
//! preflight gate that rejects an out-of-schema options block *before any method runs*.
//! [`invoke_method`] then drives one lifecycle method through whichever adapter the manifest selects
//! (`BinaryProvider` or `FlowProvider`), a fresh run id per invocation so a `FlowProvider`'s recursed
//! `PipelineRunner` run has its own identity.
//!
//! A failed method surfaces as an [`ErrorCategory::Environment`](tmx_core::ErrorCategory) error, which
//! the CLI maps to exit 5 — distinct from a pipeline `RunFailure` (exit 1).

use tmx_adapters::provider::{
    LoadedProviderManifest, build_provider, load_manifest, validate_environment_options,
};

use tmx_core::ports::driven::{IdGenerator, ProviderMethod, ProviderOutcome};
use tmx_core::{ErrorCategory, Ports, RunConfig, RunError};
use tmx_schema::Environment;

use crate::compose::Composed;

/// Load, schema-validate, and options-check the provider named by `environment.provider`.
///
/// Returns the [`LoadedProviderManifest`] ready for [`invoke_method`], or a typed error: an
/// `environment` error when no provider is declared, a `resolution`/`validation` error when the
/// manifest does not resolve/validate, or a `validation` error (`provider_options_invalid`) when the
/// options block violates the manifest's `optionsSchema`.
///
/// # Errors
///
/// See above — every failure is a typed [`RunError`].
pub async fn load_provider(
    environment: &Environment,
    composed: &Composed,
) -> Result<LoadedProviderManifest, RunError> {
    let reference = environment.provider.as_deref().ok_or_else(|| {
        RunError::new(
            ErrorCategory::Environment,
            "no_provider",
            "the Flow's environment declares no `provider` to drive",
        )
    })?;
    let ports = composed.ports();
    let loaded = load_manifest(
        reference,
        ports.reference_resolver,
        ports.source_loader,
        ports.schema,
    )
    .await?;
    // The preflight gate: reject an out-of-schema `environment.options` before any method runs.
    validate_environment_options(&loaded, environment, ports.schema)?;
    Ok(loaded)
}

/// Invoke one provider lifecycle `method` against `environment`, through the adapter the manifest
/// selects. A fresh run id is minted per call so a `FlowProvider`'s recursed run is self-identifying.
///
/// # Errors
///
/// Returns an [`ErrorCategory::Environment`](tmx_core::ErrorCategory) [`RunError`] when the method
/// fails — the exit-5 category, distinct from a pipeline `RunFailure`.
pub async fn invoke_method(
    loaded: &LoadedProviderManifest,
    composed: &Composed,
    environment: &Environment,
    method: ProviderMethod,
) -> Result<ProviderOutcome, RunError> {
    invoke_with(loaded, composed, environment, method, composed.ports()).await
}

/// Invoke a *teardown* lifecycle method (`clean`/`destroy`) best-effort after a run — through the
/// never-triggered teardown ports, so a `FlowProvider` teardown completes even when the run itself was
/// cancelled (`--timeout`/SIGINT) and its own token is hard-cancelled (06 §Ephemeral lifecycle:
/// "`clean`/`destroy` run best-effort even after a cancelled or failed run").
///
/// # Errors
///
/// Returns an [`ErrorCategory::Environment`](tmx_core::ErrorCategory) [`RunError`] when the method fails.
pub async fn invoke_teardown(
    loaded: &LoadedProviderManifest,
    composed: &Composed,
    environment: &Environment,
    method: ProviderMethod,
) -> Result<ProviderOutcome, RunError> {
    invoke_with(
        loaded,
        composed,
        environment,
        method,
        composed.teardown_ports(),
    )
    .await
}

/// Drive one provider `method` against `environment` through the given `ports` bundle, minting a fresh
/// run id so a `FlowProvider`'s recursed run is self-identifying.
async fn invoke_with(
    loaded: &LoadedProviderManifest,
    composed: &Composed,
    environment: &Environment,
    method: ProviderMethod,
    ports: Ports<'_>,
) -> Result<ProviderOutcome, RunError> {
    let run_id = composed.ids().new_run_id();
    let provider = build_provider(loaded, ports, RunConfig::default(), run_id);
    provider.invoke(method, environment).await
}
