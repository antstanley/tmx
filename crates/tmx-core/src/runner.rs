//! The `PipelineRunner` — the bounded, sequential task loop at the heart of the engine.
//!
//! [`PipelineRunner::run`] turns a [`ResolvedFlow`] into a terminal [`Pipeline`] by walking its tasks
//! in order ([04 §Pipeline execution algorithm](../../../.specs/04-execution-engine.md)): for each
//! task it gates on `if`, resolves context and requested secrets, interpolates the config, dispatches
//! through the [`TaskDispatcher`](crate::dispatch) seam, normalises and merges the output, and applies
//! the `continueOnError`-vs-abort error policy — all while enforcing the runner's load-bearing
//! invariants (state is always a JSON object, task names are unique, the index stays in range, the
//! [`Masker`] registry is populated before any event leaves, `depth ≤ FLOW_DEPTH_MAX` at every `flow`
//! recursion, and the run ends in a terminal status).
//!
//! It composes the pure Task-07…10 services — the interpolator, the [`MatcherEngine`], the
//! [`Masker`], and the state [`StateBuilder`] merge — over the driven port bundle ([`Ports`]); the
//! runner adds sequencing and policy, it never forks those units' behaviour. It is `async` because it
//! awaits driven ports, but holds **no I/O of its own**: every effect crosses a port, so the crate
//! stays inside the purity boundary even though the loop is `async`.

use std::future::Future;
use std::pin::Pin;

use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::EnvMap;
use tmx_schema::context::{Context, SecretValue};
use tmx_schema::flow::ContextRef;
use tmx_schema::limits::{FLOW_DEPTH_MAX, TASKS_PER_FLOW_MAX};
use tmx_schema::task::{FlowWith, Task};

use crate::dispatch::{Dispatch, dispatch_task, interp_template, interp_value, is_truthy};
use crate::error::RunError;
use crate::hooks::{HookKind, HookRunner};
use crate::interpolate::evaluate;
use crate::mask::Masker;
use crate::merge::{StateBuilder, normalize_output};
use crate::model::{
    Event, Milliseconds, Pipeline, ResolvedFlow, RunId, RunStatus, Scope, Severity, TaskResult,
    TaskStatus, Timestamp,
};
use crate::ports::driven::{
    ChatModel, Clock, EventSink, FileSystem, HttpClient, ObjectStore, ProcessRunner,
    ReferenceResolver, SchemaValidator, SecretResolver, SourceLoader,
};
use crate::resolve::{merged_inputs, resolve_flow};

/// The driven-port bundle the runner is generic over — a set of borrowed port handles.
///
/// Holding each capability as a `&dyn Port` reference keeps the runner injectable with any adapter
/// set (the built-in adapters, or the testkit fakes) while staying object-safe; the whole bundle is
/// `Copy` (a handful of pointers), so it threads cheaply through the recursive loop. The generic
/// [`Scheduler`](crate::ports::driven::Scheduler) is deliberately absent — the sequential runner does
/// no fan-out, so it never needs it.
#[derive(Clone, Copy)]
pub struct Ports<'a> {
    /// Runs `exec`/`run` tasks.
    pub process: &'a dyn ProcessRunner,
    /// Performs `fetch` requests.
    pub http: &'a dyn HttpClient,
    /// Performs `file` operations.
    pub file: &'a dyn FileSystem,
    /// Performs `store` operations.
    pub store: &'a dyn ObjectStore,
    /// Calls `chat-completion` models.
    pub chat: &'a dyn ChatModel,
    /// The wall-clock / duration source (the determinism seam).
    pub clock: &'a dyn Clock,
    /// The masked event stream sink.
    pub events: &'a dyn EventSink,
    /// Resolves `secretSource`s to their values.
    pub secrets: &'a dyn SecretResolver,
    /// Validates task output against a `produces` schema.
    pub schema: &'a dyn SchemaValidator,
    /// Resolves a `flow` task's `use` reference (for bounded recursion).
    pub reference_resolver: &'a dyn ReferenceResolver,
    /// Loads a referenced sub-flow's source (for bounded recursion).
    pub source_loader: &'a dyn SourceLoader,
}

/// Whether — and how strictly — a task's `produces` schema is checked at run time (04 §`produces`
/// conformance). Off by default: the seam exists but does not run unless `--check-produces` selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProducesCheck {
    /// The flag is absent: outputs are not checked at run time (the default).
    #[default]
    Off,
    /// `--check-produces[=warn]`: a mismatch is a non-blocking diagnostic.
    Warn,
    /// `--check-produces=strict`: a mismatch fails the task.
    Strict,
}

/// The engine flags that shape a run — the state cap, the global error policy, and the `produces`
/// conformance mode. Small and defaulted so the surface stays stable as flags are added.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunConfig {
    /// A global `continueOnError`: when set, every task's failure is recorded and the loop continues.
    pub continue_on_error: bool,
    /// The runtime `produces` conformance mode.
    pub check_produces: ProducesCheck,
    /// A narrowed state-size cap (clamped to the hard [`STATE_SIZE_MAX_BYTES`] ceiling); the default
    /// cap applies when `None`.
    ///
    /// [`STATE_SIZE_MAX_BYTES`]: tmx_schema::limits::STATE_SIZE_MAX_BYTES
    pub max_state_size_bytes: Option<u64>,
}

/// The bounded sequential task loop.
///
/// `in_hook` records whether this runner is executing a lifecycle-hook body. A hook body runs through
/// the same runner one level deep ([04 §Lifecycle hooks](../../../.specs/04-execution-engine.md)), but
/// **never fires lifecycle hooks of its own** — so a runner with `in_hook == true` walks its tasks
/// without firing `create`/`change`/`destroy`/`error`. This is the structural half of the
/// no-hook-inside-a-hook guarantee; [`HookRunner::fire`] carries the asserted backstop.
#[derive(Debug, Clone, Copy, Default)]
pub struct PipelineRunner {
    config: RunConfig,
    in_hook: bool,
}

/// The terminal result of a [`PipelineRunner::run`]: the [`Pipeline`] plus the names of the tasks
/// whose merge changed the state.
///
/// `changed_tasks` is the `change`-hook trigger signal (04 §Pipeline execution algorithm, step 8):
/// this unit *exposes* which state-changing tasks would fire the `change` hook, in order, but does
/// **not** itself fire it — hook execution is a later unit's obligation.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutcome {
    /// The terminal Pipeline (final state, terminal status, per-task results).
    pub pipeline: Pipeline,
    /// The names of the tasks whose merge changed the state, in execution order.
    pub changed_tasks: Vec<String>,
}

/// The internal result of running a task list: the terminal [`Pipeline`], the abort failure (when the
/// run stopped on a non-`continueOnError` task), and the names of the tasks that changed the state
/// (the `change`-hook trigger signal Task 12 consumes — this unit exposes it, it does not fire hooks).
pub(crate) struct Outcome {
    pub(crate) pipeline: Pipeline,
    pub(crate) failure: Option<RunError>,
    pub(crate) changed: Vec<String>,
}

/// What one task step produced: it was skipped by its `if` gate, or it produced a normalised output.
enum StepOutcome {
    Skipped,
    Produced(Value),
}

impl PipelineRunner {
    /// A runner with the default engine flags (no global `continueOnError`, no `produces` check,
    /// default state cap).
    #[must_use]
    pub fn new(config: RunConfig) -> Self {
        Self {
            config,
            in_hook: false,
        }
    }

    /// A runner that executes a lifecycle-hook body one level deep: same engine flags, but it walks
    /// its tasks without firing any lifecycle hook of its own (the no-hook-inside-a-hook guarantee).
    #[must_use]
    pub(crate) fn for_hook_body(config: RunConfig) -> Self {
        Self {
            config,
            in_hook: true,
        }
    }

    /// Run `flow` end to end: emit `run.start`, walk the tasks, emit `run.finish`, and return the
    /// terminal [`Pipeline`].
    ///
    /// `id` is the run id (minted by the caller via the `IdGenerator` port), `inputs` the resolved
    /// `inputs.*` scope object, `masker` the run's secret registry (populated as tasks resolve their
    /// secrets), `resolved_secrets` the accumulating list of resolved secret values (for the
    /// registry-populated boundary assertion), and `depth` the `flow`-recursion depth (0 at the top).
    ///
    /// A run whose task fails without `continueOnError` returns `Ok` with a `failed` status and the
    /// failing [`TaskResult`] recorded — the terminal record, not an `Err`. `Err` is reserved for a
    /// pre-flight abort (a missing/duplicate task name, too many tasks, an unsupported reference) and
    /// for an output-port write failure.
    #[allow(clippy::too_many_arguments)] // a run is defined by exactly these collaborators; a bag struct would only hide them
    pub async fn run(
        &self,
        id: &RunId,
        flow: &ResolvedFlow,
        inputs: &Value,
        ports: Ports<'_>,
        masker: &mut Masker,
        resolved_secrets: &mut Vec<String>,
        depth: u32,
    ) -> Result<RunOutcome, RunError> {
        validate_task_names(flow)?;
        let flow_name = flow.name.clone().unwrap_or_else(|| "flow".to_string());
        let start_ms = ports.clock.now_ms();
        emit_event(
            ports,
            masker,
            resolved_secrets,
            Event::RunStart {
                id: id.clone(),
                flow: flow_name,
            },
        )
        .await?;

        // The hook runner reads this Flow's `context.hooks`; it is a no-op when no hook is declared or
        // when this runner is itself a hook body (`in_hook`), so a hook-free flow runs exactly as
        // before hooks existed.
        let hooks = HookRunner::new(flow.context.as_ref(), self.config, self.in_hook);

        // `create` fires once on entry to `running`, right after `run.start`.
        hooks
            .fire(
                HookKind::Create,
                id,
                inputs,
                ports,
                masker,
                resolved_secrets,
                depth,
            )
            .await?;

        let outcome = self
            .run_tasks(
                id,
                flow,
                inputs,
                ports,
                masker,
                resolved_secrets,
                depth,
                Some(&hooks),
            )
            .await?;

        let status = if outcome.failure.is_some() {
            RunStatus::Failed
        } else {
            RunStatus::Ok
        };

        // `error` fires when a task aborted the Pipeline (a real failure, not a `continueOnError`
        // record); it precedes `destroy`, which is the lifecycle's `finally`.
        if outcome.failure.is_some() {
            hooks
                .fire(
                    HookKind::Error,
                    id,
                    inputs,
                    ports,
                    masker,
                    resolved_secrets,
                    depth,
                )
                .await?;
        }
        // `destroy` fires on every terminal status — success, failure, or cancellation — like a
        // `finally`. It is status-independent: the run reaches this point exactly once, terminally.
        hooks
            .fire(
                HookKind::Destroy,
                id,
                inputs,
                ports,
                masker,
                resolved_secrets,
                depth,
            )
            .await?;

        let total = Milliseconds(ports.clock.now_ms().0.saturating_sub(start_ms.0));
        emit_event(
            ports,
            masker,
            resolved_secrets,
            Event::RunFinish {
                id: id.clone(),
                status,
                ms: total,
            },
        )
        .await?;

        let mut pipeline = outcome.pipeline;
        pipeline.status = status;
        // Invariant: the Pipeline never leaves a terminal status.
        assert!(
            pipeline.status.is_terminal(),
            "a finished run must hold a terminal status"
        );
        Ok(RunOutcome {
            pipeline,
            changed_tasks: outcome.changed,
        })
    }

    /// Walk the task list, returning the [`Outcome`]. A boxed future so the `flow`-recursion cycle
    /// (`run_tasks` → step → sub-flow → `run_tasks`) has a type-erased indirection and stays a finite
    /// size, satisfying Tiger Style's no-unbounded-recursion rule together with the `FLOW_DEPTH_MAX`
    /// guard.
    ///
    /// `hooks` is `Some` only for the Pipeline's own top-level task loop, where a state-changing task
    /// fires the `change` hook; it is `None` for a sub-flow's loop and for a hook body, so `change`
    /// fires once per state-changing task of the Pipeline and never per sub-flow or per hook task.
    #[allow(clippy::too_many_arguments)] // mirrors `run`'s collaborators; threaded through the recursion
    pub(crate) fn run_tasks<'a>(
        &'a self,
        id: &'a RunId,
        flow: &'a ResolvedFlow,
        inputs: &'a Value,
        ports: Ports<'a>,
        masker: &'a mut Masker,
        resolved_secrets: &'a mut Vec<String>,
        depth: u32,
        hooks: Option<&'a HookRunner<'a>>,
    ) -> Pin<Box<dyn Future<Output = Result<Outcome, RunError>> + Send + 'a>> {
        Box::pin(async move {
            let count = flow.tasks.len();
            // The loop is bounded by TASKS_PER_FLOW_MAX (checked as a typed error in
            // `validate_task_names`); assert the bound as a backstop before iterating.
            assert!(
                count as u64 <= u64::from(TASKS_PER_FLOW_MAX),
                "task count must be within TASKS_PER_FLOW_MAX"
            );
            let mut builder = match self.config.max_state_size_bytes {
                Some(cap) => StateBuilder::with_cap(cap),
                None => StateBuilder::new(),
            };
            let mut results: Vec<TaskResult> = Vec::with_capacity(count);
            let mut changed: Vec<String> = Vec::new();
            let mut failure: Option<RunError> = None;

            for (index, task) in flow.tasks.iter().enumerate() {
                // Invariant: the task index stays in range for the bounded loop.
                assert!(index < count, "task index must stay in range");
                let name = task.name.as_deref().unwrap_or("");
                // Invariant: names were validated non-empty and unique before the loop.
                assert!(!name.is_empty(), "task name must be non-empty");
                // Invariant: the state is always a JSON object between tasks.
                assert!(
                    builder.as_value().is_object(),
                    "the Pipeline state must be a JSON object"
                );
                let key = task.output.as_deref().unwrap_or(name).to_string();
                assert!(
                    !key.is_empty(),
                    "the merge key (output ?? name) is non-empty"
                );

                let started_at = ports.clock.now();
                let start_ms = ports.clock.now_ms();

                let step = self
                    .run_step(
                        id,
                        task,
                        name,
                        flow,
                        inputs,
                        builder.as_value(),
                        ports,
                        masker,
                        resolved_secrets,
                        depth,
                    )
                    .await;

                let elapsed = Milliseconds(ports.clock.now_ms().0.saturating_sub(start_ms.0));

                match step {
                    Ok(StepOutcome::Skipped) => {
                        emit_event(
                            ports,
                            masker,
                            resolved_secrets,
                            Event::TaskSkip {
                                name: name.to_string(),
                                reason: "if=false".to_string(),
                            },
                        )
                        .await?;
                        results.push(TaskResult {
                            name: name.to_string(),
                            status: TaskStatus::Skipped,
                            output: None,
                            error: None,
                            started_at,
                            ms: elapsed,
                        });
                    }
                    Ok(StepOutcome::Produced(output)) => {
                        let previous = builder
                            .as_value()
                            .as_object()
                            .and_then(|o| o.get(&key))
                            .cloned();
                        match builder.merge(&key, output.clone(), name) {
                            Ok(()) => {
                                // `change` fires once per state-changing task, and only when the merge
                                // actually changed the state — a task whose merge is a no-op does not
                                // fire it (04 §Lifecycle hooks).
                                let did_change = previous.as_ref() != Some(&output);
                                if did_change {
                                    changed.push(name.to_string());
                                }
                                emit_event(
                                    ports,
                                    masker,
                                    resolved_secrets,
                                    Event::TaskFinish {
                                        name: name.to_string(),
                                        status: TaskStatus::Ok,
                                        ms: elapsed,
                                        output: Some(output.clone()),
                                    },
                                )
                                .await?;
                                results.push(TaskResult {
                                    name: name.to_string(),
                                    status: TaskStatus::Ok,
                                    output: Some(output),
                                    error: None,
                                    started_at,
                                    ms: elapsed,
                                });
                                // Fire `change` after the task's own `task.finish`, so a reader can
                                // attribute the hook to its triggering task.
                                if did_change && let Some(hooks) = hooks {
                                    hooks
                                        .fire(
                                            HookKind::Change,
                                            id,
                                            inputs,
                                            ports,
                                            masker,
                                            resolved_secrets,
                                            depth,
                                        )
                                        .await?;
                                }
                            }
                            Err(merge_err) => {
                                // A cap/depth overflow at merge aborts the run regardless of the
                                // per-task error policy (04 §State size cap).
                                failure = self
                                    .fail_task(
                                        name,
                                        &key,
                                        false,
                                        merge_err,
                                        started_at,
                                        elapsed,
                                        ports,
                                        masker,
                                        resolved_secrets,
                                        &mut builder,
                                        &mut results,
                                    )
                                    .await?;
                            }
                        }
                    }
                    Err(step_err) => {
                        let continue_on_error = task.continue_on_error.unwrap_or(false)
                            || self.config.continue_on_error;
                        failure = self
                            .fail_task(
                                name,
                                &key,
                                continue_on_error,
                                step_err,
                                started_at,
                                elapsed,
                                ports,
                                masker,
                                resolved_secrets,
                                &mut builder,
                                &mut results,
                            )
                            .await?;
                    }
                }

                if failure.is_some() {
                    break;
                }
            }

            let status = if failure.is_some() {
                RunStatus::Failed
            } else {
                RunStatus::Ok
            };
            let mut pipeline = Pipeline::new(id.clone());
            pipeline.state = builder.into_state();
            pipeline.status = status;
            pipeline.results = results;
            Ok(Outcome {
                pipeline,
                failure,
                changed,
            })
        })
    }

    /// Run one task through its lifecycle up to (but not including) the merge: gate, resolve context
    /// and secrets, emit `task.start`, dispatch, normalise, and check `produces`.
    #[allow(clippy::too_many_arguments)] // the step's collaborators; a bag struct would only hide them
    async fn run_step(
        &self,
        id: &RunId,
        task: &Task,
        name: &str,
        flow: &ResolvedFlow,
        inputs: &Value,
        state: &Value,
        ports: Ports<'_>,
        masker: &mut Masker,
        resolved_secrets: &mut Vec<String>,
        depth: u32,
    ) -> Result<StepOutcome, RunError> {
        let (ctx_env, secret_defs) = resolve_context(flow.context.as_ref(), task)?;
        let env_value = env_to_value(&ctx_env);
        let empty = Value::Object(serde_json::Map::new());

        // `if` gate — evaluated with no secrets bound (the gate cannot read secrets).
        if let Some(condition) = &task.if_condition {
            let gate_scope = Scope {
                inputs,
                env: &env_value,
                secrets: &empty,
                tasks: state,
                item: None,
                case: None,
                output: None,
                matrix: &empty,
            };
            // `if` accepts both a bare expression (`inputs.enabled`) and a `${{ … }}`-wrapped one; a
            // wrapped form is interpolated (a lone `${{ expr }}` keeps its value's type), a bare form
            // is evaluated directly rather than being read as a literal string.
            let gate = if condition.contains("${{") {
                interp_template(condition, &gate_scope)?
            } else {
                evaluate(condition, &gate_scope)?
            };
            if !is_truthy(&gate) {
                return Ok(StepOutcome::Skipped);
            }
        }

        // Resolve only the secrets the task lists; each resolved value is registered with the Masker
        // before any dispatch, so nothing can leave unmasked.
        let secrets_value =
            resolve_secrets(task, &secret_defs, ports, masker, resolved_secrets).await?;

        let scope = Scope {
            inputs,
            env: &env_value,
            secrets: &secrets_value,
            tasks: state,
            item: None,
            case: None,
            output: None,
            matrix: &empty,
        };

        emit_event(
            ports,
            masker,
            resolved_secrets,
            Event::TaskStart {
                name: name.to_string(),
            },
        )
        .await?;

        match dispatch_task(task, name, &scope, ports, &ctx_env, depth).await? {
            Dispatch::Leaf(adapter) => {
                let output = normalize_output(adapter);
                self.check_produces(task, &output, ports, name)?;
                Ok(StepOutcome::Produced(output))
            }
            Dispatch::Flow(fw) => {
                let output = self
                    .run_subflow(id, fw, &scope, ports, masker, resolved_secrets, depth)
                    .await?;
                Ok(StepOutcome::Produced(output))
            }
        }
    }

    /// Recurse into a `flow` task: load and resolve the referenced sub-flow, run it one level deeper,
    /// and return its final state as this task's output. The depth guard already passed in
    /// [`dispatch_task`]; the assertion here is the backstop.
    #[allow(clippy::too_many_arguments)] // recursion collaborators; threaded through the loop
    async fn run_subflow(
        &self,
        id: &RunId,
        fw: &FlowWith,
        scope: &Scope<'_>,
        ports: Ports<'_>,
        masker: &mut Masker,
        resolved_secrets: &mut Vec<String>,
        depth: u32,
    ) -> Result<Value, RunError> {
        // Backstop for the negative-space guard in `dispatch_task`. Equivalent to the spec's
        // `depth + 1 <= FLOW_DEPTH_MAX` bound (this recursion runs at `depth + 1`).
        assert!(
            depth < FLOW_DEPTH_MAX,
            "flow recursion must stay within FLOW_DEPTH_MAX"
        );
        let resolved = ports.reference_resolver.resolve(&fw.use_ref).await?;
        let source = ports
            .source_loader
            .load(&resolved.path, resolved.kind)
            .await?;
        let sub_flow = resolve_flow(source)?;
        validate_task_names(&sub_flow)?;
        let sub_inputs = match &fw.inputs {
            None => Value::Object(serde_json::Map::new()),
            Some(value) => interp_value(value, scope)?,
        };
        let merged = merged_inputs(&sub_inputs, &sub_flow.inputs);
        // A sub-flow's loop does not fire the Pipeline's `change` hook per inner task: `change` is a
        // property of the Pipeline's own task loop, so `hooks` is `None` here.
        let outcome = self
            .run_tasks(
                id,
                &sub_flow,
                &merged,
                ports,
                masker,
                resolved_secrets,
                depth + 1,
                None,
            )
            .await?;
        if let Some(failure) = outcome.failure {
            return Err(failure);
        }
        Ok(outcome.pipeline.state.into_value())
    }

    /// The `produces` conformance seam (04 §`produces` conformance). Off by default — it does not run
    /// when the flag is absent; under `warn` a mismatch is a non-blocking diagnostic; under `strict` a
    /// mismatch fails the task.
    fn check_produces(
        &self,
        task: &Task,
        output: &Value,
        ports: Ports<'_>,
        name: &str,
    ) -> Result<(), RunError> {
        let Some(schema) = &task.produces else {
            return Ok(());
        };
        match self.config.check_produces {
            ProducesCheck::Off => Ok(()),
            ProducesCheck::Warn => {
                // A warn-level mismatch is a diagnostic the reporter surfaces (a later unit); the run
                // is unaffected. Validation itself must not fault.
                let _ = ports.schema.validate_produces(output, schema)?;
                Ok(())
            }
            ProducesCheck::Strict => {
                let diagnostics = ports.schema.validate_produces(output, schema)?;
                if diagnostics
                    .iter()
                    .any(|d| matches!(d.severity, Severity::Error))
                {
                    Err(RunError::run_failure(
                        "produces_mismatch",
                        format!("task {name:?} output does not conform to its produces schema"),
                    )
                    .with_task(name))
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Record a task failure and decide whether the run stops. Emits `task.error` (masked), then under
    /// `continue_on_error` records the error in the task's state slot and returns `None` (continue);
    /// otherwise records the failing [`TaskResult`] and returns the abort failure (stop).
    #[allow(clippy::too_many_arguments)] // records across the event stream, state, and results at once
    async fn fail_task(
        &self,
        name: &str,
        key: &str,
        continue_on_error: bool,
        err: RunError,
        started_at: Timestamp,
        elapsed: Milliseconds,
        ports: Ports<'_>,
        masker: &Masker,
        resolved_secrets: &[String],
        builder: &mut StateBuilder,
        results: &mut Vec<TaskResult>,
    ) -> Result<Option<RunError>, RunError> {
        let _ = self;
        emit_event(
            ports,
            masker,
            resolved_secrets,
            Event::TaskError {
                name: name.to_string(),
                error: err.clone(),
            },
        )
        .await?;

        if continue_on_error {
            let slot =
                serde_json::json!({ "error": serde_json::to_value(&err).unwrap_or(Value::Null) });
            match builder.merge(key, slot.clone(), name) {
                Ok(()) => {
                    results.push(TaskResult {
                        name: name.to_string(),
                        status: TaskStatus::Error,
                        output: Some(slot),
                        error: Some(err),
                        started_at,
                        ms: elapsed,
                    });
                    Ok(None)
                }
                // Even the error slot cannot fit under the cap: escalate to a hard stop.
                Err(merge_err) => {
                    results.push(TaskResult {
                        name: name.to_string(),
                        status: TaskStatus::Error,
                        output: None,
                        error: Some(err),
                        started_at,
                        ms: elapsed,
                    });
                    Ok(Some(merge_err))
                }
            }
        } else {
            results.push(TaskResult {
                name: name.to_string(),
                status: TaskStatus::Error,
                output: None,
                error: Some(err.clone()),
                started_at,
                ms: elapsed,
            });
            Ok(Some(err))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Free helpers — masking-aware emission, context/secret resolution, and name validation.
// ---------------------------------------------------------------------------------------------

/// Emit one event through the sink, redacting its payload and asserting the Masker registry holds
/// every resolved secret first — the *registry-populated* half of the boundary guarantee.
pub(crate) async fn emit_event(
    ports: Ports<'_>,
    masker: &Masker,
    resolved_secrets: &[String],
    event: Event,
) -> Result<(), RunError> {
    let refs: Vec<&str> = resolved_secrets.iter().map(String::as_str).collect();
    // Negative space: nothing leaves the core until every resolved secret is registered.
    masker.assert_ready(&refs);
    // Seal the event as a `Masked<Event>` (redacting `task.finish` output / `task.error` message and
    // stamping the run Masker's origin); the sink asserts that origin before it emits.
    let masked = masker.redact_event(&event);
    ports.events.emit(&masked).await
}

/// Resolve the effective context for a task: the inherited (Flow) env/secrets combined with the
/// task's own inline context per `contextStrategy` (`merge`/`replace`) and `contextPrecedence`
/// (`local`/`inherited`), each section independently.
type ResolvedContext = (EnvMap, IndexMap<String, SecretValue>);

fn resolve_context(flow_ctx: Option<&Context>, task: &Task) -> Result<ResolvedContext, RunError> {
    let inherited_env = flow_ctx.and_then(|c| c.env.clone()).unwrap_or_default();
    let inherited_secrets = flow_ctx.and_then(|c| c.secrets.clone()).unwrap_or_default();

    let local_ctx = match &task.context {
        None => None,
        Some(ContextRef::Inline(ctx)) => Some(ctx.as_ref()),
        Some(ContextRef::Reference(_)) => {
            return Err(RunError::resolution(
                "unsupported_reference",
                "a referenced task context is resolved by the loading unit, not the runner",
            )
            .with_task(task.name.as_deref().unwrap_or_default()));
        }
    };

    let Some(local_ctx) = local_ctx else {
        return Ok((inherited_env, inherited_secrets));
    };

    let replace = task.context_strategy.as_deref() == Some("replace");
    let inherited_wins = task.context_precedence.as_deref() == Some("inherited");

    let env = combine_section(
        inherited_env,
        local_ctx.env.clone().unwrap_or_default(),
        replace,
        inherited_wins,
    );
    let secrets = combine_section(
        inherited_secrets,
        local_ctx.secrets.clone().unwrap_or_default(),
        replace,
        inherited_wins,
    );
    Ok((env, secrets))
}

/// Combine one context section (env or secrets): `replace` swaps the inherited map for the local one
/// (when the local is non-empty); otherwise the two are unioned, with `inherited_wins` deciding a key
/// collision.
fn combine_section<V: Clone>(
    inherited: IndexMap<String, V>,
    local: IndexMap<String, V>,
    replace: bool,
    inherited_wins: bool,
) -> IndexMap<String, V> {
    if replace {
        return if local.is_empty() { inherited } else { local };
    }
    let mut out = inherited;
    for (key, value) in local {
        if inherited_wins && out.contains_key(&key) {
            continue;
        }
        out.insert(key, value);
    }
    out
}

/// Resolve the secrets a task requested (only the names in `task.secrets`), registering each with the
/// Masker and recording it in `resolved_secrets`. A requested name absent from the resolved context
/// is a typed `unknown_secret`; an unrequested name is never touched.
async fn resolve_secrets(
    task: &Task,
    secret_defs: &IndexMap<String, SecretValue>,
    ports: Ports<'_>,
    masker: &mut Masker,
    resolved_secrets: &mut Vec<String>,
) -> Result<Value, RunError> {
    let mut map = serde_json::Map::new();
    if let Some(names) = &task.secrets {
        for requested in names {
            let def = secret_defs.get(requested).ok_or_else(|| {
                RunError::resolution(
                    "unknown_secret",
                    format!("task requested secret {requested:?} not defined in context"),
                )
                .with_task(task.name.as_deref().unwrap_or_default())
            })?;
            let value = match def {
                SecretValue::Literal(literal) => literal.clone(),
                SecretValue::Source(source) => ports.secrets.resolve(source).await?,
            };
            // Single choke point for every resolver (literal, provider, any future seam): an empty
            // resolved secret cannot be registered with the Masker (it skips empty registrations by
            // design), so it would slip downstream unmaskable and later trip `masker.assert_ready`.
            // Reject it typed here — before `register`/push — so the run fails cleanly instead of
            // panicking.
            if value.is_empty() {
                return Err(RunError::resolution(
                    "secret_value_empty",
                    format!("task requested secret {requested:?} resolved to an empty value"),
                )
                .with_task(task.name.as_deref().unwrap_or_default()));
            }
            masker.register(value.clone());
            resolved_secrets.push(value.clone());
            map.insert(requested.clone(), Value::String(value));
        }
    }
    Ok(Value::Object(map))
}

/// Project a resolved env map into the `env.*` scope value.
fn env_to_value(env: &EnvMap) -> Value {
    Value::Object(
        env.iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect(),
    )
}

/// Assert the load-bearing name invariants as typed pre-flight errors (and an assertion backstop):
/// every task has a non-empty name, names are unique, and the count is within
/// [`TASKS_PER_FLOW_MAX`].
fn validate_task_names(flow: &ResolvedFlow) -> Result<(), RunError> {
    if flow.tasks.len() as u64 > u64::from(TASKS_PER_FLOW_MAX) {
        return Err(RunError::validation(
            "too_many_tasks",
            format!("a flow may hold at most {TASKS_PER_FLOW_MAX} tasks"),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for task in &flow.tasks {
        let name = task.name.as_deref().unwrap_or("");
        if name.is_empty() {
            return Err(RunError::validation(
                "missing_task_name",
                "every task must have a non-empty name before the run",
            ));
        }
        if !seen.insert(name.to_string()) {
            return Err(RunError::validation(
                "duplicate_task_name",
                format!("duplicate task name {name:?}"),
            ));
        }
    }
    // Backstop for the "task names are unique" invariant.
    assert_eq!(
        seen.len(),
        flow.tasks.len(),
        "task names must be unique (backstop)"
    );
    Ok(())
}
