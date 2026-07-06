//! The use-case implementations — the concrete bodies behind the driving port traits.
//!
//! This unit lands [`EngineRunFlow`], the [`RunFlow`](crate::ports::driving::RunFlow) use case that
//! wires the load → resolve → run → mask pipeline over the driven port bundle: it resolves the Flow
//! reference through the `ReferenceResolver`/`SourceLoader` ports, resolves it into a
//! [`ResolvedFlow`], mints a run id, drives the [`PipelineRunner`] loop, and folds the terminal
//! [`Pipeline`] into a masked [`RunRecord`] — the final state and every event scrubbed by the run's
//! [`Masker`]. It also lands [`EngineLintFlow`], the [`LintFlow`](crate::ports::driving::LintFlow) use
//! case — the deeper static pass (resolution + dataflow) behind `tmx lint`. The remaining use cases
//! (validate/inspect/…) arrive with their own units.

use std::collections::BTreeSet;

use async_trait::async_trait;
use serde_json::Value;
use tmx_schema::Flow;
use tmx_schema::limits::FLOW_DEPTH_MAX;

use crate::error::RunError;
use crate::lint::analyze_flow;
use crate::mask::Masker;
use crate::model::{Diagnostic, PipelineState, RunRecord, Severity};
use crate::ports::driven::IdGenerator;
use crate::ports::driving::{LintFlow, RunFlow, RunOptions};
use crate::preflight::PreflightPorts;
use crate::resolve::{merged_inputs, resolve_flow};
use crate::runner::{PipelineRunner, Ports, RunConfig};

/// The engine's [`RunFlow`] use case: load → resolve → run → mask, over a driven port bundle.
///
/// Holds the [`Ports`] bundle, the [`IdGenerator`] (which is not part of the loop's bundle — a run id
/// is minted once, up front), and the [`RunConfig`] engine flags. Borrows its ports, so the
/// composition root (or a test) owns the adapters and hands the use case their references.
pub struct EngineRunFlow<'a> {
    ports: Ports<'a>,
    ids: &'a dyn IdGenerator,
    config: RunConfig,
}

impl<'a> EngineRunFlow<'a> {
    /// Wire the use case over `ports`, the `ids` generator, and the engine `config`.
    #[must_use]
    pub fn new(ports: Ports<'a>, ids: &'a dyn IdGenerator, config: RunConfig) -> Self {
        Self { ports, ids, config }
    }
}

#[async_trait]
impl RunFlow for EngineRunFlow<'_> {
    async fn run(
        &self,
        reference: &str,
        inputs: Value,
        options: RunOptions,
    ) -> Result<RunRecord, RunError> {
        // `RunOptions`' loader/reporter flags are the composition root's concern; the engine flags
        // this use case honours are the wired `RunConfig`. The global `continueOnError` flag arrives
        // via config, not `RunOptions`, so the options only gate future load-time behaviour here.
        let _ = &options;

        // Load → resolve.
        let resolved_source = self.ports.reference_resolver.resolve(reference).await?;
        let source = self
            .ports
            .source_loader
            .load(&resolved_source.path, resolved_source.kind)
            .await?;
        let flow = resolve_flow(source)?;
        let merged = merged_inputs(&inputs, &flow.inputs);

        // Run.
        let id = self.ids.new_run_id();
        let started_at = self.ports.clock.now();
        let start_ms = self.ports.clock.now_ms();
        let mut masker = Masker::new();
        let mut resolved_secrets: Vec<String> = Vec::new();
        let runner = PipelineRunner::new(self.config);
        let pipeline = runner
            .run(
                &id,
                &flow,
                &merged,
                self.ports,
                &mut masker,
                &mut resolved_secrets,
                None,
                0,
            )
            .await?
            .pipeline;
        let finished_at = self.ports.clock.now();
        let total_ms =
            crate::model::Milliseconds(self.ports.clock.now_ms().0.saturating_sub(start_ms.0));

        // Mask the final state through the run's Masker before it leaves the core.
        let masked_state = masker
            .redact_value(pipeline.state.as_value())
            .into_inner()
            .into_owned();
        // Total: the state stays an object across the merge, so re-wrapping cannot fail; fall back to
        // an empty state rather than take a panicking path.
        let final_state =
            PipelineState::new(masked_state).unwrap_or_else(|_| PipelineState::empty());

        Ok(RunRecord {
            id,
            flow: flow.name.clone().or_else(|| Some(reference.to_string())),
            status: pipeline.status,
            started_at,
            finished_at: Some(finished_at),
            ms: Some(total_ms),
            final_state: Some(final_state),
            results: pipeline.results,
        })
    }
}

/// The engine's [`LintFlow`] use case — the deeper static pass (resolution + dataflow) behind
/// `tmx lint` ([03 §`lint`](../../../.specs/03-loading-and-preflight.md)).
///
/// Over the three preflight ports (resolve → load → validate), it (1) resolves and loads the Flow,
/// (2) confirms its `environment` / `context` references load and, where the provider manifest carries
/// an `optionsSchema`, checks the environment's `options` against it, (3) walks the `flow`-task import
/// graph to detect a cyclic import, and (4) runs the pure [`analyze_flow`] dataflow pass (typo'd
/// `produces` reads, undeclared inputs, unlisted secrets, duplicate/missing task names). Every finding
/// is a warning [`Diagnostic`]; the CLI decides whether `--strict` promotes it to an exit-`3` error.
pub struct EngineLintFlow<'a> {
    ports: PreflightPorts<'a>,
}

impl<'a> EngineLintFlow<'a> {
    /// Wire the lint use case over the preflight port bundle (resolve → load → validate).
    #[must_use]
    pub fn new(ports: PreflightPorts<'a>) -> Self {
        Self { ports }
    }

    /// Resolve and load the source at `reference` into its JSON model, propagating a loader/resolver
    /// failure as its typed [`RunError`].
    async fn load(&self, reference: &str) -> Result<(String, Value), RunError> {
        let resolved = self.ports.reference_resolver.resolve(reference).await?;
        let value = self
            .ports
            .source_loader
            .load(&resolved.path, resolved.kind)
            .await?;
        Ok((resolved.path, value))
    }

    /// Confirm the top Flow's `environment` / `context` references load, and — where a resolved
    /// provider manifest carries an `optionsSchema` — check the environment's `options` against it.
    async fn check_env_and_context(
        &self,
        flow: &Value,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), RunError> {
        // Resolve a referenced context to confirm it loads (its inline body is not analysed here).
        if let Some(reference) = flow.get("context").and_then(Value::as_str) {
            self.confirm_loads(reference, "context", diagnostics).await;
        }
        // The environment is either an inline object or a string reference; either way, obtain its
        // inline value so the provider `optionsSchema` check can run against it.
        let environment = match flow.get("environment") {
            Some(Value::String(reference)) => {
                match self
                    .confirm_loads(reference, "environment", diagnostics)
                    .await
                {
                    Some(value) => value,
                    None => return Ok(()),
                }
            }
            Some(value @ Value::Object(_)) => value.clone(),
            _ => return Ok(()),
        };
        self.check_environment_options(&environment, diagnostics)
            .await
    }

    /// Resolve and load `reference`, returning its value; a failure is a warning `unresolved_reference`
    /// diagnostic (not a hard error — lint reports and continues) and `None`.
    async fn confirm_loads(
        &self,
        reference: &str,
        role: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<Value> {
        match self.load(reference).await {
            Ok((_, value)) => Some(value),
            Err(error) => {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "unresolved_reference",
                        format!(
                            "the {role} reference `{reference}` does not load: {}",
                            error.message
                        ),
                    )
                    .with_path(reference.to_string()),
                );
                None
            }
        }
    }

    /// Where an inline `environment` declares a `provider` (a reference) and an `options` block, load
    /// the provider manifest and, if it carries an `optionsSchema`, check the options against it,
    /// surfacing any schema violation as a warning `provider_options_invalid` diagnostic.
    async fn check_environment_options(
        &self,
        environment: &Value,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), RunError> {
        let (Some(provider_ref), Some(options)) = (
            environment.get("provider").and_then(Value::as_str),
            environment.get("options"),
        ) else {
            return Ok(());
        };
        let Some((_, manifest)) = self.load(provider_ref).await.ok() else {
            // A dangling provider reference is already reported by the capability/preflight path; the
            // lint pass does not double-report it here.
            return Ok(());
        };
        let Some(options_schema) = manifest.get("optionsSchema") else {
            return Ok(());
        };
        let violations = self
            .ports
            .schema
            .validate_produces(options, options_schema)?;
        for violation in violations {
            if matches!(violation.severity, Severity::Error) {
                diagnostics.push(Diagnostic::new(
                    Severity::Warning,
                    "provider_options_invalid",
                    format!(
                        "`environment.options` violates the provider's optionsSchema: {}",
                        violation.message
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Walk the `flow`-task import graph from `flow`, detecting a cyclic import — a `use` reference that
    /// resolves back to a source already on the current chain. A cycle is a warning
    /// `cyclic_flow_import` diagnostic; recursion is bounded by [`FLOW_DEPTH_MAX`] as a backstop.
    async fn check_imports(
        &self,
        flow: &Value,
        chain: &mut Vec<String>,
        depth: u32,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), RunError> {
        if depth >= FLOW_DEPTH_MAX {
            return Ok(());
        }
        for use_ref in flow_import_refs(flow) {
            let Ok(resolved) = self.ports.reference_resolver.resolve(&use_ref).await else {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "unresolved_reference",
                        format!("the imported flow reference `{use_ref}` does not resolve"),
                    )
                    .with_path(use_ref.clone()),
                );
                continue;
            };
            if chain.contains(&resolved.path) {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "cyclic_flow_import",
                        format!(
                            "the flow import `{use_ref}` forms a cycle back to `{}`",
                            resolved.path
                        ),
                    )
                    .with_path(use_ref.clone()),
                );
                continue;
            }
            let Ok(sub_flow) = self
                .ports
                .source_loader
                .load(&resolved.path, resolved.kind)
                .await
            else {
                continue;
            };
            chain.push(resolved.path);
            Box::pin(self.check_imports(&sub_flow, chain, depth + 1, diagnostics)).await?;
            chain.pop();
        }
        Ok(())
    }
}

#[async_trait]
impl LintFlow for EngineLintFlow<'_> {
    async fn lint(&self, reference: &str) -> Result<Vec<Diagnostic>, RunError> {
        let (path, value) = self.load(reference).await?;
        // A document that does not even parse as a Flow is a hard `validation` error — the same depth
        // `validate` fails at; lint's added depth (resolution + dataflow) only runs over a parseable
        // Flow.
        let _flow: Flow = serde_json::from_value(value.clone()).map_err(|error| {
            RunError::validation(
                "flow_parse_error",
                format!("could not parse `{path}`: {error}"),
            )
        })?;

        let mut diagnostics = Vec::new();
        // The pure dataflow pass: typo'd produces reads, undeclared inputs, unlisted secrets, and
        // duplicate/missing task names.
        analyze_flow(&value, &mut diagnostics);
        // Resolution depth: confirm references load, check provider options, detect cyclic imports.
        self.check_env_and_context(&value, &mut diagnostics).await?;
        let mut chain = vec![path];
        self.check_imports(&value, &mut chain, 0, &mut diagnostics)
            .await?;
        Ok(diagnostics)
    }
}

/// The `use` references of every top-level `flow`-type task in `flow`, in source order — the edges of
/// the import graph the cycle check walks.
fn flow_import_refs(flow: &Value) -> Vec<String> {
    let raw_tasks: Vec<&Value> = match flow.get("tasks") {
        Some(Value::Array(list)) => list.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    };
    let mut refs = Vec::new();
    let mut seen = BTreeSet::new();
    for task in raw_tasks {
        if task.get("type").and_then(Value::as_str) == Some("flow")
            && let Some(use_ref) = task
                .get("with")
                .and_then(|w| w.get("use"))
                .and_then(Value::as_str)
            && seen.insert(use_ref.to_string())
        {
            refs.push(use_ref.to_string());
        }
    }
    refs
}
