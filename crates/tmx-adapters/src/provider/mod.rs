//! The [`EnvironmentProvider`] adapters — `BinaryProvider` and `FlowProvider`.
//!
//! An environment provider materialises a Flow's `environment` block into real resources through the
//! four lifecycle methods (`bootstrap`/`deploy`/`clean`/`destroy`,
//! [`.specs/06-ports-and-adapters.md` §Environment and provider execution](../../../../.specs/06-ports-and-adapters.md)).
//! The [`ProviderManifest`] declares which of the two adapters materialises it:
//!
//! - [`BinaryProvider`] invokes the manifest's `binary` with the method's subcommand string, handing
//!   the resolved [`Environment`] to it as JSON on stdin; the process result is the method result.
//! - [`FlowProvider`] runs the method's inline tasks / referenced Flow **through the same
//!   [`PipelineRunner`]** the engine uses — a provider method body *is* a Flow, so it inherits the
//!   whole task model and the [`FLOW_DEPTH_MAX`](tmx_schema::limits::FLOW_DEPTH_MAX) recursion bound.
//!
//! Both satisfy the [`EnvironmentProvider`] port, so the lifecycle caller (the `tmx env` command and
//! the `tmx run` ephemeral wrapper) never sees which adapter is in play. A failed method is an
//! [`ErrorCategory::Environment`] [`RunError`] — never a `RunFailure` — so the CLI maps it to exit 5,
//! distinct from a pipeline failure (exit 1).
//!
//! [`load_manifest`] resolves and schema-validates a manifest through the loader/resolver ports, and
//! [`validate_environment_options`] checks a Flow's `environment.options` against the manifest's
//! `optionsSchema` — the preflight gate that rejects an out-of-schema options block before any method
//! runs. [`build_provider`] selects the adapter from the manifest `type`.

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Value, json};

use tmx_core::error::{ErrorCategory, RunError};
use tmx_core::mask::Masker;
use tmx_core::model::{Milliseconds, ResolvedFlow, RunId, RunStatus, Severity};
use tmx_core::ports::driven::{
    ArtifactKind, EnvironmentProvider, ProcessKind, ProcessRunner, ProcessSpec, ProviderMethod,
    ProviderOutcome, ReferenceResolver, SchemaValidator, SourceLoader,
};
use tmx_core::runner::{PipelineRunner, Ports, RunConfig};
use tmx_core::{merged_inputs, resolve_flow};
use tmx_schema::{Environment, ProviderManifest, ProviderMethodBody, ProviderType};

/// A resolved, schema-validated provider manifest plus the raw JSON it was parsed from.
///
/// The raw JSON is retained because a `flow`-provider method body carrying inline tasks is run as a
/// Flow: wrapping the raw method body in `{ "tasks": … }` and resolving it reuses the engine's own
/// Flow resolution, avoiding a re-serialisation of the typed (deserialise-only) task model.
#[derive(Debug, Clone)]
pub struct LoadedProviderManifest {
    /// The typed manifest — provider `type`, `binary`, `optionsSchema`, and the four method bodies.
    pub manifest: ProviderManifest,
    /// The manifest's raw JSON, for extracting a `flow`-method's inline task body verbatim.
    pub raw: Value,
    /// The reference the manifest was loaded from (for diagnostics).
    pub reference: String,
}

/// Resolve and load the provider manifest at `reference` (a filesystem reference in v0), validating
/// it against the provider schema before parsing it into a [`ProviderManifest`].
///
/// # Errors
///
/// Returns a `resolution` [`RunError`] for an unresolved reference, or a `validation` error
/// (`provider_manifest_invalid` / `provider_manifest_parse_error`) for a manifest that fails schema
/// validation or does not parse.
pub async fn load_manifest(
    reference: &str,
    resolver: &dyn ReferenceResolver,
    loader: &dyn SourceLoader,
    schema: &dyn SchemaValidator,
) -> Result<LoadedProviderManifest, RunError> {
    let resolved = resolver.resolve(reference).await?;
    let raw = loader.load(&resolved.path, resolved.kind).await?;
    let diagnostics = schema.validate(&raw, ArtifactKind::Provider)?;
    if let Some(error) = diagnostics
        .iter()
        .find(|d| matches!(d.severity, Severity::Error))
    {
        return Err(RunError::validation(
            "provider_manifest_invalid",
            format!(
                "provider manifest `{}` failed schema validation: {}",
                resolved.path, error.message
            ),
        )
        .with_path(resolved.path.clone()));
    }
    let manifest: ProviderManifest = serde_json::from_value(raw.clone()).map_err(|e| {
        RunError::validation(
            "provider_manifest_parse_error",
            format!("could not parse provider manifest `{}`: {e}", resolved.path),
        )
        .with_path(resolved.path.clone())
    })?;
    Ok(LoadedProviderManifest {
        manifest,
        raw,
        reference: reference.to_string(),
    })
}

/// Validate a Flow's `environment.options` against the manifest's `optionsSchema` — the preflight
/// gate (06 §Environment and provider execution: "The CLI validates `environment.options` against the
/// provider's `optionsSchema` first").
///
/// Only the inline-object `optionsSchema` form is validated in v0; a string (`$ref`/path) form is a
/// registry seam and is treated as "no local schema to check". An absent `options` block is validated
/// as an empty object, so an `optionsSchema` requiring a field rejects a Flow that omits `options`.
///
/// # Errors
///
/// Returns a `validation` [`RunError`] (`provider_options_invalid`) when the options block violates
/// the schema, before any provider method runs.
pub fn validate_environment_options(
    loaded: &LoadedProviderManifest,
    environment: &Environment,
    schema: &dyn SchemaValidator,
) -> Result<(), RunError> {
    let Some(options_schema) = &loaded.manifest.options_schema else {
        return Ok(());
    };
    if !options_schema.is_object() {
        // A string `$ref`/path optionsSchema is out of scope for v0 in-process validation.
        return Ok(());
    }
    let options = environment.options.clone().unwrap_or_else(|| json!({}));
    let diagnostics = schema.validate_produces(&options, options_schema)?;
    if let Some(error) = diagnostics
        .iter()
        .find(|d| matches!(d.severity, Severity::Error))
    {
        return Err(RunError::validation(
            "provider_options_invalid",
            format!(
                "environment.options does not satisfy provider `{}` optionsSchema: {}",
                loaded.manifest.name, error.message
            ),
        ));
    }
    Ok(())
}

/// Build the [`EnvironmentProvider`] adapter the manifest `type` selects, over the wired ports.
///
/// A `binary` manifest yields a [`BinaryProvider`] (needing only the [`ProcessRunner`]); a `flow`
/// manifest yields a [`FlowProvider`] (needing the full [`Ports`] bundle, the engine [`RunConfig`],
/// and a run id to drive a [`PipelineRunner`]). The returned box borrows `loaded` and the ports, so
/// both must outlive the lifecycle.
#[must_use]
pub fn build_provider<'a>(
    loaded: &'a LoadedProviderManifest,
    ports: Ports<'a>,
    config: RunConfig,
    run_id: RunId,
) -> Box<dyn EnvironmentProvider + 'a> {
    match loaded.manifest.provider_type {
        ProviderType::Binary => Box::new(BinaryProvider {
            loaded,
            process: ports.process,
        }),
        ProviderType::Flow => Box::new(FlowProvider {
            loaded,
            ports,
            config,
            run_id,
        }),
    }
}

/// Wrap a downstream failure as an [`ErrorCategory::Environment`] method failure — the mapping that
/// keeps a provider method's failure exit 5 (`environment`), distinct from a pipeline `RunFailure`.
fn method_failed(provider: &str, method: &str, detail: &str) -> RunError {
    RunError::new(
        ErrorCategory::Environment,
        "provider_method_failed",
        format!("provider `{provider}` method `{method}` failed: {detail}"),
    )
    .with_task(method.to_string())
}

/// Interpret a process's captured stdout as the method result: parse it as JSON when it is JSON,
/// else wrap the (lossy-UTF-8) text as `{ "stdout": <text> }` so the outcome is always a JSON value.
fn parse_process_output(stdout: &[u8]) -> Value {
    serde_json::from_slice::<Value>(stdout)
        .unwrap_or_else(|_| json!({ "stdout": String::from_utf8_lossy(stdout).into_owned() }))
}

/// Invokes a manifest binary with each method's subcommand — the `binary`-provider adapter.
pub struct BinaryProvider<'a> {
    /// The loaded manifest (its `binary` and per-method subcommand strings).
    loaded: &'a LoadedProviderManifest,
    /// The process runner the subcommand is executed through.
    process: &'a dyn ProcessRunner,
}

#[async_trait]
impl EnvironmentProvider for BinaryProvider<'_> {
    async fn invoke(
        &self,
        method: ProviderMethod,
        environment: &Environment,
    ) -> Result<ProviderOutcome, RunError> {
        let name = method.as_str();
        let provider = self.loaded.manifest.name.as_str();
        // The method body must be a subcommand string for a binary provider.
        let subcommand = match self.loaded.manifest.methods.body(name) {
            Some(ProviderMethodBody::Ref(subcommand)) => subcommand.clone(),
            _ => {
                return Err(RunError::new(
                    ErrorCategory::Environment,
                    "provider_method_not_subcommand",
                    format!(
                        "binary provider `{provider}` method `{name}` must be a subcommand string"
                    ),
                )
                .with_task(name.to_string()));
            }
        };
        let binary = self.loaded.manifest.binary.as_deref().ok_or_else(|| {
            RunError::new(
                ErrorCategory::Environment,
                "provider_binary_missing",
                format!("binary provider `{provider}` declares no `binary` executable"),
            )
        })?;
        let command = if subcommand.trim().is_empty() {
            binary.to_string()
        } else {
            format!("{binary} {subcommand}")
        };
        // The resolved environment (image/resources/options) is handed to the binary on stdin as JSON.
        let stdin = serde_json::to_string(environment).map_err(|e| {
            method_failed(
                provider,
                name,
                &format!("could not serialise environment: {e}"),
            )
        })?;
        let spec = ProcessSpec {
            kind: ProcessKind::Exec,
            command,
            language: None,
            args: Vec::new(),
            env: IndexMap::new(),
            cwd: None,
            stdin: Some(stdin),
            timeout: None,
        };
        match self.process.run(spec).await {
            Ok(output) => Ok(ProviderOutcome {
                method,
                output: parse_process_output(&output.stdout),
                ms: output.ms,
            }),
            // A non-zero exit / spawn error is a `RunFailure` from the process runner; re-categorise
            // it as an environment failure so the CLI maps it to exit 5, not exit 1.
            Err(error) => Err(method_failed(provider, name, &error.message)),
        }
    }
}

/// Runs each method's inline tasks / referenced Flow through the shared [`PipelineRunner`] — the
/// `flow`-provider adapter.
pub struct FlowProvider<'a> {
    /// The loaded manifest (its per-method Flow bodies, typed and raw).
    loaded: &'a LoadedProviderManifest,
    /// The full driven-port bundle the recursed [`PipelineRunner`] runs over.
    ports: Ports<'a>,
    /// The engine flags a method's Flow inherits.
    config: RunConfig,
    /// The run id the provider lifecycle runs emit under.
    run_id: RunId,
}

impl FlowProvider<'_> {
    /// Build the [`ResolvedFlow`] (and merged inputs) a method body runs as: an inline task
    /// collection becomes a one-Flow `{ tasks: … }`; a `Ref`/`use` body loads and resolves the
    /// referenced Flow through the loader/resolver ports.
    async fn method_flow(
        &self,
        name: &str,
        body: &ProviderMethodBody,
    ) -> Result<(ResolvedFlow, Value), RunError> {
        let provider = self.loaded.manifest.name.as_str();
        match body {
            ProviderMethodBody::Tasks(_) => {
                let raw_body = self
                    .loaded
                    .raw
                    .get("methods")
                    .and_then(|methods| methods.get(name))
                    .cloned()
                    .ok_or_else(|| {
                        method_failed(provider, name, "method body missing from manifest JSON")
                    })?;
                let flow = resolve_flow(json!({ "tasks": raw_body }))
                    .map_err(|e| method_failed(provider, name, &e.message))?;
                let inputs = merged_inputs(&json!({}), &flow.inputs);
                Ok((flow, inputs))
            }
            ProviderMethodBody::Ref(reference) => {
                let flow = self.load_flow(reference).await?;
                let inputs = merged_inputs(&json!({}), &flow.inputs);
                Ok((flow, inputs))
            }
            ProviderMethodBody::Use(import) => {
                let flow = self.load_flow(&import.use_ref).await?;
                let supplied = import.inputs.clone().unwrap_or_else(|| json!({}));
                let inputs = merged_inputs(&supplied, &flow.inputs);
                Ok((flow, inputs))
            }
        }
    }

    /// Load and resolve a referenced Flow through the loader/resolver ports.
    async fn load_flow(&self, reference: &str) -> Result<ResolvedFlow, RunError> {
        let resolved = self.ports.reference_resolver.resolve(reference).await?;
        let source = self
            .ports
            .source_loader
            .load(&resolved.path, resolved.kind)
            .await?;
        resolve_flow(source)
    }
}

#[async_trait]
impl EnvironmentProvider for FlowProvider<'_> {
    async fn invoke(
        &self,
        method: ProviderMethod,
        _environment: &Environment,
    ) -> Result<ProviderOutcome, RunError> {
        let name = method.as_str();
        let provider = self.loaded.manifest.name.as_str();
        let Some(body) = self.loaded.manifest.methods.body(name) else {
            return Err(method_failed(provider, name, "unknown lifecycle method"));
        };

        let (flow, inputs) = self.method_flow(name, body).await?;

        let start_ms = self.ports.clock.now_ms();
        let mut masker = Masker::new();
        let mut resolved_secrets: Vec<String> = Vec::new();
        // The method body runs through the *same* PipelineRunner at depth 0, so any nested `flow`
        // task inside it recurses under the FLOW_DEPTH_MAX bound exactly as an ordinary run would. Its
        // `map`/`eval` fan-out runs over the always-available serial scheduler (correct, deterministic
        // output — identical to the concurrent adapter, only serial).
        let scheduler = crate::scheduler::SerialScheduler::new();
        let runner = PipelineRunner::new(self.config);
        let outcome = runner
            .run(
                &self.run_id,
                &flow,
                &inputs,
                self.ports,
                &scheduler,
                &mut masker,
                &mut resolved_secrets,
                None,
                0,
            )
            .await
            .map_err(|e| method_failed(provider, name, &e.message))?;
        let ms = Milliseconds(self.ports.clock.now_ms().0.saturating_sub(start_ms.0));

        // A method's Flow that ends non-`ok` is a *method* failure — an environment error (exit 5),
        // not the pipeline `RunFailure` (exit 1) the inner task raised.
        if outcome.pipeline.status != RunStatus::Ok {
            let detail = outcome
                .pipeline
                .results
                .iter()
                .rev()
                .find_map(|result| result.error.as_ref())
                .map_or_else(|| "a method task failed".to_string(), |e| e.message.clone());
            return Err(method_failed(provider, name, &detail));
        }

        // Mask the method's final state before it leaves the adapter, mirroring the run boundary.
        let output = masker
            .redact_value(outcome.pipeline.state.as_value())
            .into_inner()
            .into_owned();
        Ok(ProviderOutcome { method, output, ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use tmx_core::ports::driven::ProcessOutput;

    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    /// A scripted [`ProcessRunner`] whose next call either succeeds with fixed stdout or fails with a
    /// `RunFailure` (as the real OS runner does on a non-zero exit).
    struct ScriptedProcess {
        stdout: Vec<u8>,
        fail: bool,
    }

    #[async_trait]
    impl ProcessRunner for ScriptedProcess {
        async fn run(&self, _spec: ProcessSpec) -> Result<ProcessOutput, RunError> {
            if self.fail {
                Err(RunError::run_failure(
                    "process_exit_nonzero",
                    "process exited with non-zero status 1",
                ))
            } else {
                Ok(ProcessOutput {
                    exit_code: Some(0),
                    stdout: self.stdout.clone(),
                    stderr: Vec::new(),
                    ms: Milliseconds(3),
                })
            }
        }
    }

    fn binary_manifest() -> LoadedProviderManifest {
        let raw = json!({
            "name": "ecs",
            "type": "binary",
            "binary": "tmx-ecs",
            "methods": {
                "bootstrap": "bootstrap",
                "deploy": "deploy",
                "clean": "clean",
                "destroy": "destroy"
            }
        });
        let manifest: ProviderManifest =
            serde_json::from_value(raw.clone()).expect("manifest parses");
        LoadedProviderManifest {
            manifest,
            raw,
            reference: "ecs.provider.yaml".to_string(),
        }
    }

    fn sample_environment() -> Environment {
        serde_json::from_value(json!({ "name": "dev", "provider": "ecs", "image": "alpine:3" }))
            .expect("environment deserialises")
    }

    #[test]
    fn binary_provider_returns_the_process_output_as_the_method_result() {
        let loaded = binary_manifest();
        let process = ScriptedProcess {
            stdout: br#"{"deployed":true}"#.to_vec(),
            fail: false,
        };
        let provider = BinaryProvider {
            loaded: &loaded,
            process: &process,
        };
        let outcome =
            block_on_ready(provider.invoke(ProviderMethod::Deploy, &sample_environment()))
                .expect("a zero-exit binary method succeeds");
        assert_eq!(outcome.method, ProviderMethod::Deploy, "the method echoes");
        assert_eq!(
            outcome.output,
            json!({ "deployed": true }),
            "the process stdout parses into the method result"
        );
    }

    #[test]
    fn binary_provider_maps_a_failed_process_to_an_environment_error_not_a_run_failure() {
        // The process runner returns a `RunFailure` on a non-zero exit; the provider must re-categorise
        // it to `Environment` so the CLI maps a failed method to exit 5, distinct from a pipeline
        // failure (exit 1).
        let loaded = binary_manifest();
        let process = ScriptedProcess {
            stdout: Vec::new(),
            fail: true,
        };
        let provider = BinaryProvider {
            loaded: &loaded,
            process: &process,
        };
        let error = block_on_ready(provider.invoke(ProviderMethod::Deploy, &sample_environment()))
            .expect_err("a non-zero-exit binary method fails");
        assert_eq!(
            error.category,
            ErrorCategory::Environment,
            "a failed provider method is an Environment error (exit 5), not a RunFailure (exit 1)"
        );
        assert_eq!(
            error.code, "provider_method_failed",
            "the failure carries the stable method-failed code"
        );
        assert_eq!(
            error.task.as_deref(),
            Some("deploy"),
            "the error names the failing method"
        );
    }

    #[test]
    fn binary_provider_rejects_a_non_subcommand_method_body() {
        // Negative space: a binary provider whose method is inline tasks (not a subcommand string) is
        // a typed environment error, never a panic or a silent no-op.
        let raw = json!({
            "name": "bad",
            "type": "binary",
            "binary": "x",
            "methods": {
                "bootstrap": "b",
                "deploy": [ { "name": "t", "type": "exec", "with": { "command": "echo hi" } } ],
                "clean": "c",
                "destroy": "d"
            }
        });
        let manifest: ProviderManifest = serde_json::from_value(raw.clone()).expect("parses");
        let loaded = LoadedProviderManifest {
            manifest,
            raw,
            reference: "bad.yaml".to_string(),
        };
        let process = ScriptedProcess {
            stdout: Vec::new(),
            fail: false,
        };
        let provider = BinaryProvider {
            loaded: &loaded,
            process: &process,
        };
        let error = block_on_ready(provider.invoke(ProviderMethod::Deploy, &sample_environment()))
            .expect_err("a task-body binary method is rejected");
        assert_eq!(
            error.code, "provider_method_not_subcommand",
            "a non-subcommand binary body names its own code"
        );
        assert_eq!(
            error.category,
            ErrorCategory::Environment,
            "and is an environment-category error"
        );
    }

    #[test]
    fn validate_environment_options_rejects_an_out_of_schema_options_block() {
        // The optionsSchema requires `cluster`; an options block without it is rejected before any
        // method runs (the preflight negative-space gate).
        let raw = json!({
            "name": "ecs",
            "type": "binary",
            "binary": "x",
            "optionsSchema": { "type": "object", "required": ["cluster"] },
            "methods": { "bootstrap": "b", "deploy": "d", "clean": "c", "destroy": "x" }
        });
        let manifest: ProviderManifest = serde_json::from_value(raw.clone()).expect("parses");
        let loaded = LoadedProviderManifest {
            manifest,
            raw,
            reference: "ecs.yaml".to_string(),
        };
        let schema = crate::validate::JsonSchemaValidator::new().expect("schema compiles");

        // Missing the required `cluster`: rejected.
        let bad_env: Environment =
            serde_json::from_value(json!({ "provider": "ecs", "options": { "region": "eu" } }))
                .expect("environment deserialises");
        let error = validate_environment_options(&loaded, &bad_env, &schema)
            .expect_err("an out-of-schema options block is rejected");
        assert_eq!(
            error.code, "provider_options_invalid",
            "the rejection names the options-invalid code"
        );

        // A conforming options block passes.
        let good_env: Environment = serde_json::from_value(
            json!({ "provider": "ecs", "options": { "cluster": "main", "region": "eu" } }),
        )
        .expect("environment deserialises");
        assert!(
            validate_environment_options(&loaded, &good_env, &schema).is_ok(),
            "an options block satisfying the schema is accepted"
        );
    }

    #[test]
    fn parse_process_output_falls_back_to_a_stdout_wrapper_for_non_json() {
        // The `flow`-provider's method-running path (recursion into the shared PipelineRunner) is
        // exercised end to end by the CLI integration tests (`tests/cli_env.rs`), which drive a real
        // flow provider through the composed adapter bundle; here we pin the stdout normalisation.
        assert_eq!(
            parse_process_output(br#"{"a":1}"#),
            json!({ "a": 1 }),
            "JSON stdout parses through"
        );
        assert_eq!(
            parse_process_output(b"not json"),
            json!({ "stdout": "not json" }),
            "non-JSON stdout is wrapped so the result is always a JSON value"
        );
    }
}
