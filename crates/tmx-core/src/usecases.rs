//! The use-case implementations — the concrete bodies behind the driving port traits.
//!
//! This unit lands [`EngineRunFlow`], the [`RunFlow`](crate::ports::driving::RunFlow) use case that
//! wires the load → resolve → run → mask pipeline over the driven port bundle: it resolves the Flow
//! reference through the `ReferenceResolver`/`SourceLoader` ports, resolves it into a
//! [`ResolvedFlow`], mints a run id, drives the [`PipelineRunner`] loop, and folds the terminal
//! [`Pipeline`] into a masked [`RunRecord`] — the final state and every event scrubbed by the run's
//! [`Masker`]. The remaining use cases (validate/lint/inspect/…) arrive with their own units.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::RunError;
use crate::mask::Masker;
use crate::model::{PipelineState, RunRecord};
use crate::ports::driven::IdGenerator;
use crate::ports::driving::{RunFlow, RunOptions};
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
