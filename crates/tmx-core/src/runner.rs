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
use tmx_schema::limits::{CONCURRENCY_MAX, FANOUT_WIDTH_MAX, FLOW_DEPTH_MAX, TASKS_PER_FLOW_MAX};
use tmx_schema::task::{EvalWith, FlowWith, MapWith, Task, TaskWith};

use crate::cancel::{CancelReason, CancelToken};
use crate::dispatch::{Dispatch, dispatch_task, interp_template, interp_value, is_truthy};
use crate::error::RunError;
use crate::fanout::{run_eval, run_map};
use crate::hooks::{HookKind, HookRunner};
use crate::interpolate::evaluate;
use crate::mask::Masker;
use crate::merge::{StateBuilder, normalize_output};
use crate::model::{
    Event, Milliseconds, Pipeline, PipelineState, ResolvedFlow, RunId, RunStatus, Scope, Severity,
    TaskResult, TaskStatus, Timestamp,
};
use crate::ports::driven::{
    ChatModel, Clock, EventSink, FileSystem, HttpClient, ObjectStore, ProcessRunner,
    ReferenceResolver, Scheduler, SchemaValidator, SecretResolver, SourceLoader,
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
    /// The run's cancellation token — threaded from the root into every adapter call and awaited
    /// alongside the work ([`CancelToken::guard`]). A never-triggered token is a no-op, so a run with
    /// no `--timeout` and no interrupt threads it yet behaves exactly as before.
    pub cancel: &'a CancelToken,
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
    /// The run's global `map`/`eval` fan-out concurrency ceiling (the `--concurrency` flag): every
    /// fan-out task's own `concurrency` is further clamped by this. `None` leaves the engine ceiling
    /// [`CONCURRENCY_MAX`](tmx_schema::limits::CONCURRENCY_MAX) as the only bound, so a fan-out task's
    /// `concurrency` governs exactly as before this flag existed.
    pub concurrency_cap: Option<u32>,
}

/// The bounded sequential task loop.
///
/// `in_hook` records whether this runner is executing a lifecycle-hook body. A hook body runs through
/// the same runner one level deep ([04 §Lifecycle hooks](../../../.specs/04-execution-engine.md)), but
/// **never fires lifecycle hooks of its own** — so a runner with `in_hook == true` walks its tasks
/// without firing `create`/`change`/`destroy`/`error`. This is the structural half of the
/// no-hook-inside-a-hook guarantee; [`HookRunner::fire`] carries the asserted backstop.
#[derive(Debug, Clone)]
pub struct PipelineRunner {
    config: RunConfig,
    in_hook: bool,
    /// The `--matrix` combination bound as `${{ matrix.<key> }}` for every task of the run — and,
    /// because it is carried on the runner, for its sub-flows and hook bodies too (07 §Matrix sugar).
    /// An empty object when the run carries no matrix, so a matrix-free run reads `matrix` as `{}`
    /// exactly as before this flag existed. Not `Copy` (a `Value` is heap-backed), so the runner is
    /// cloned, never bit-copied.
    matrix: Value,
}

/// The empty `matrix` binding a matrix-free run carries — an object, never `null`, so
/// `${{ matrix }}` is always a bound namespace (never an undefined-key resolution error).
fn empty_matrix() -> Value {
    Value::Object(serde_json::Map::new())
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
    /// The cancellation reason, when the run was cancelled (`--timeout` / SIGINT) rather than
    /// completing or failing on a task. Distinct from `failure`: a cancelled run ends `timed_out` /
    /// `cancelled`, not `failed`, and does not fire the `error` hook.
    pub(crate) cancelled: Option<CancelReason>,
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
            matrix: empty_matrix(),
        }
    }

    /// A runner that executes a lifecycle-hook body one level deep: same engine flags, but it walks
    /// its tasks without firing any lifecycle hook of its own (the no-hook-inside-a-hook guarantee).
    #[must_use]
    pub(crate) fn for_hook_body(config: RunConfig) -> Self {
        Self {
            config,
            in_hook: true,
            matrix: empty_matrix(),
        }
    }

    /// Bind a `--matrix` combination for this run: every task's `${{ matrix.<key> }}` reads from
    /// `matrix` (07 §Matrix sugar). A non-object `matrix` is treated as no binding (an empty object),
    /// so the `matrix` namespace is always a bound object.
    #[must_use]
    pub fn with_matrix(mut self, matrix: Value) -> Self {
        self.matrix = if matrix.is_object() {
            matrix
        } else {
            empty_matrix()
        };
        self
    }

    /// Run `flow` end to end: emit `run.start`, walk the tasks, emit `run.finish`, and return the
    /// terminal [`Pipeline`].
    ///
    /// `id` is the run id (minted by the caller via the `IdGenerator` port), `inputs` the resolved
    /// `inputs.*` scope object, `masker` the run's secret registry (populated as tasks resolve their
    /// secrets), `resolved_secrets` the accumulating list of resolved secret values (for the
    /// registry-populated boundary assertion), `seed` the optional prior Pipeline state to start the
    /// task loop from (`--state-in`; `None` starts from `{}`), and `depth` the `flow`-recursion depth
    /// (0 at the top).
    ///
    /// A run whose task fails without `continueOnError` returns `Ok` with a `failed` status and the
    /// failing [`TaskResult`] recorded — the terminal record, not an `Err`. `Err` is reserved for a
    /// pre-flight abort (a missing/duplicate task name, too many tasks, an unsupported reference) and
    /// for an output-port write failure.
    #[allow(clippy::too_many_arguments)] // a run is defined by exactly these collaborators; a bag struct would only hide them
    pub async fn run<S: Scheduler>(
        &self,
        id: &RunId,
        flow: &ResolvedFlow,
        inputs: &Value,
        ports: Ports<'_>,
        scheduler: &S,
        masker: &mut Masker,
        resolved_secrets: &mut Vec<String>,
        seed: Option<&PipelineState>,
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
        let hooks = HookRunner::new(
            flow.context.as_ref(),
            self.config,
            self.in_hook,
            self.matrix.clone(),
        );

        // `create` fires once on entry to `running`, right after `run.start`.
        hooks
            .fire(
                HookKind::Create,
                id,
                inputs,
                ports,
                scheduler,
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
                scheduler,
                masker,
                resolved_secrets,
                seed,
                depth,
                Some(&hooks),
            )
            .await?;

        // A cancelled run ends `timed_out` / `cancelled`; otherwise a task abort is `failed` and a
        // clean walk is `ok`. Cancellation is terminal on its own — it is not a task `failure`.
        let status = match outcome.cancelled {
            Some(reason) => reason.to_status(),
            None if outcome.failure.is_some() => RunStatus::Failed,
            None => RunStatus::Ok,
        };

        // The lifecycle `finally` (`destroy`, and on a real failure `error`) must run *best-effort*
        // even after a cancelled run — but the run's own token is now (hard-)cancelled, so firing a
        // hook body through it would insta-cancel the teardown. Fire the teardown hooks through a
        // fresh, never-triggered token so `destroy` actually runs to completion. Struct-update keeps
        // every other port handle; only the cancel token is swapped.
        let teardown_token = CancelToken::new();
        let teardown_ports = Ports {
            cancel: &teardown_token,
            ..ports
        };

        // `error` fires when a task aborted the Pipeline (a real failure, not a `continueOnError`
        // record, and not a cancellation); it precedes `destroy`, which is the lifecycle's `finally`.
        if outcome.failure.is_some() && outcome.cancelled.is_none() {
            hooks
                .fire(
                    HookKind::Error,
                    id,
                    inputs,
                    teardown_ports,
                    scheduler,
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
                teardown_ports,
                scheduler,
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
    pub(crate) fn run_tasks<'a, S: Scheduler>(
        &'a self,
        id: &'a RunId,
        flow: &'a ResolvedFlow,
        inputs: &'a Value,
        ports: Ports<'a>,
        scheduler: &'a S,
        masker: &'a mut Masker,
        resolved_secrets: &'a mut Vec<String>,
        seed: Option<&'a PipelineState>,
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
            // Seed the state from a prior run (`--state-in`) when supplied, so a sliced continuation
            // reads earlier tasks' output via `${{ tasks.NAME.field }}`; otherwise start from `{}`.
            // A narrowed `--max-state-size` cap is honoured in either case.
            let mut builder = match (seed, self.config.max_state_size_bytes) {
                (Some(state), Some(cap)) => StateBuilder::from_state_with_cap(state.clone(), cap),
                (Some(state), None) => StateBuilder::from_state(state.clone()),
                (None, Some(cap)) => StateBuilder::with_cap(cap),
                (None, None) => StateBuilder::new(),
            };
            let mut results: Vec<TaskResult> = Vec::with_capacity(count);
            let mut changed: Vec<String> = Vec::new();
            let mut failure: Option<RunError> = None;
            let mut cancelled: Option<CancelReason> = None;

            for (index, task) in flow.tasks.iter().enumerate() {
                // Cancellation (soft phase): once `--timeout`/SIGINT has requested a stop, the
                // sequential runner — the degenerate `Scheduler` — ceases to dispatch new work. The
                // in-flight task (if any) already got its grace window through the guard below.
                if let Some(reason) = ports.cancel.requested_reason() {
                    cancelled = Some(reason);
                    break;
                }
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

                // Run the step *alongside* the cancellation token: the guard awaits the adapter work
                // and, once the grace window elapses and hard cancellation fires, resolves to a typed
                // cancellation error — dropping (hard-stopping) the in-flight work so no cancelled run
                // is held hostage by an adapter that ignores the grace period.
                let step = ports
                    .cancel
                    .guard(self.run_step(
                        id,
                        task,
                        name,
                        flow,
                        inputs,
                        builder.as_value(),
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        depth,
                    ))
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
                                            scheduler,
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
                        // A cancellation stopped this task (a hard-stop of the in-flight adapter, or a
                        // cancellation propagated up from a sub-flow). It is carried by the error's
                        // category, not the per-task error policy: a cancelled run ends
                        // `timed_out`/`cancelled` and never fires the `error` hook, regardless of the
                        // task's `continueOnError`. Emit `task.error` to balance the `task.start` the
                        // step already emitted, record the cut-off task, then stop.
                        if let Some(reason) = cancel_reason_of(&step_err) {
                            emit_event(
                                ports,
                                masker,
                                resolved_secrets,
                                Event::TaskError {
                                    name: name.to_string(),
                                    error: step_err.clone(),
                                },
                            )
                            .await?;
                            results.push(TaskResult {
                                name: name.to_string(),
                                status: TaskStatus::Error,
                                output: None,
                                error: Some(step_err),
                                started_at,
                                ms: elapsed,
                            });
                            cancelled = Some(reason);
                            break;
                        }
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

            // A cancelled walk ends `timed_out`/`cancelled`; a task abort is `failed`; otherwise `ok`.
            let status = match cancelled {
                Some(reason) => reason.to_status(),
                None if failure.is_some() => RunStatus::Failed,
                None => RunStatus::Ok,
            };
            let mut pipeline = Pipeline::new(id.clone());
            pipeline.state = builder.into_state();
            pipeline.status = status;
            pipeline.results = results;
            Ok(Outcome {
                pipeline,
                failure,
                changed,
                cancelled,
            })
        })
    }

    /// Run one task through its lifecycle up to (but not including) the merge: gate, resolve context
    /// and secrets, emit `task.start`, dispatch, normalise, and check `produces`.
    #[allow(clippy::too_many_arguments)] // the step's collaborators; a bag struct would only hide them
    async fn run_step<S: Scheduler>(
        &self,
        id: &RunId,
        task: &Task,
        name: &str,
        flow: &ResolvedFlow,
        inputs: &Value,
        state: &Value,
        ports: Ports<'_>,
        scheduler: &S,
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
                item_alias: None,
                item_index: None,
                case: None,
                output: None,
                matrix: &self.matrix,
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
            item_alias: None,
            item_index: None,
            case: None,
            output: None,
            matrix: &self.matrix,
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
                    .run_subflow(
                        id,
                        fw,
                        &scope,
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        depth,
                    )
                    .await?;
                Ok(StepOutcome::Produced(output))
            }
            Dispatch::Map(mw) => {
                // The fan-out callback is `Fn` (shared, possibly concurrent), so it cannot borrow the
                // masker mutably: the map task's own secrets are already resolved and registered above,
                // and the inner task runs under the parent scope's already-bound secrets (a flow inner
                // task masks its own newly-resolved secrets locally). Hand the callback immutable
                // borrows for event emission and route to `run_map`.
                let output = self
                    .run_map_task(
                        id,
                        mw,
                        name,
                        &scope,
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        &ctx_env,
                        depth,
                    )
                    .await?;
                self.check_produces(task, &output, ports, name)?;
                Ok(StepOutcome::Produced(output))
            }
            Dispatch::Eval(ew) => {
                let output = self
                    .run_eval_task(
                        id,
                        ew,
                        name,
                        &scope,
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        &ctx_env,
                        depth,
                    )
                    .await?;
                self.check_produces(task, &output, ports, name)?;
                Ok(StepOutcome::Produced(output))
            }
        }
    }

    /// Recurse into a `flow` task: load and resolve the referenced sub-flow, run it one level deeper,
    /// and return its final state as this task's output. The depth guard already passed in
    /// [`dispatch_task`]; the assertion here is the backstop.
    #[allow(clippy::too_many_arguments)] // recursion collaborators; threaded through the loop
    async fn run_subflow<S: Scheduler>(
        &self,
        id: &RunId,
        fw: &FlowWith,
        scope: &Scope<'_>,
        ports: Ports<'_>,
        scheduler: &S,
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
                scheduler,
                masker,
                resolved_secrets,
                None,
                depth + 1,
                None,
            )
            .await?;
        // A cancelled sub-flow propagates the cancellation up as its typed error, so the parent loop
        // classifies it (by category) as a cancellation and stops the whole run — a nested `flow` can
        // never swallow a `--timeout`/SIGINT.
        if let Some(reason) = outcome.cancelled {
            return Err(reason.to_error());
        }
        if let Some(failure) = outcome.failure {
            return Err(failure);
        }
        Ok(outcome.pipeline.state.into_value())
    }

    /// Fan a `map` task out over its `items` through the injected [`Scheduler`], collecting the
    /// per-item outputs into an array in item order and emitting a `map.item.finish` per element.
    ///
    /// The inner task runs once per element under the parent `scope` with the element bound as
    /// `${{ item.* }}`, at the map task's own recursion `depth` (a `flow` inner task recurses one level
    /// deeper, guarded by [`run_map`] before any element runs). Every `run_map` guard
    /// (`fanout_too_wide`, `concurrency_too_high`, `map_items_not_array`, `flow_depth_exceeded`)
    /// surfaces unchanged through this seam.
    #[allow(clippy::too_many_arguments)] // the fan-out collaborators; threaded from the step
    async fn run_map_task<S: Scheduler>(
        &self,
        id: &RunId,
        map: &MapWith,
        name: &str,
        scope: &Scope<'_>,
        ports: Ports<'_>,
        scheduler: &S,
        masker: &Masker,
        resolved_secrets: &[String],
        ctx_env: &EnvMap,
        depth: u32,
    ) -> Result<Value, RunError> {
        let cap = self.config.concurrency_cap.unwrap_or(CONCURRENCY_MAX);
        let inner = &map.task;
        let inner_name = inner.name.as_deref().unwrap_or(name);
        run_map(
            map,
            name,
            scope,
            scheduler,
            cap,
            depth,
            |index, element, _item_depth| async move {
                let start_ms = ports.clock.now_ms();
                let item_scope = Scope {
                    item: Some(&element),
                    // The `as:` alias renames the element's root (default `item`), and the element's
                    // position is threaded so `${{ <alias>.index }}` resolves for every element type.
                    item_alias: map.as_binding.as_deref(),
                    item_index: Some(index),
                    ..*scope
                };
                let result = self
                    .run_inner_task(
                        id,
                        inner,
                        inner_name,
                        &item_scope,
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        ctx_env,
                        depth,
                    )
                    .await;
                // Emit `map.item.finish` even when the element failed (its error is recorded in-slot by
                // `run_map` under `continueOnError`, or aborts the map otherwise) so the element
                // boundary is always visible on the stream.
                let ms = Milliseconds(ports.clock.now_ms().0.saturating_sub(start_ms.0));
                emit_event(
                    ports,
                    masker,
                    resolved_secrets,
                    Event::MapItemFinish {
                        name: name.to_string(),
                        index,
                        ms,
                    },
                )
                .await?;
                result
            },
        )
        .await
    }

    /// Fan an `eval` task out over its `dataset` through the injected [`Scheduler`], returning the
    /// [`Scorecard`](crate::model::Scorecard) as a JSON value and emitting an `eval.case.finish` per
    /// scored case.
    ///
    /// The `subject` (when present) runs once per case with the case bound as `${{ case }}`; its output
    /// is what the scorers grade (`${{ output }}`). Every `run_eval` guard and the `threshold` gate
    /// (`eval_threshold_missed`) surface unchanged through this seam.
    #[allow(clippy::too_many_arguments)] // the fan-out collaborators; threaded from the step
    async fn run_eval_task<S: Scheduler>(
        &self,
        id: &RunId,
        eval: &EvalWith,
        name: &str,
        scope: &Scope<'_>,
        ports: Ports<'_>,
        scheduler: &S,
        masker: &Masker,
        resolved_secrets: &[String],
        ctx_env: &EnvMap,
        depth: u32,
    ) -> Result<Value, RunError> {
        let cap = self.config.concurrency_cap.unwrap_or(CONCURRENCY_MAX);
        let subject = eval.subject.as_ref();
        let subject_name = subject.and_then(|s| s.name.as_deref()).unwrap_or(name);
        let scorecard = run_eval(
            eval,
            name,
            scope,
            scheduler,
            ports.chat,
            ports.process,
            cap,
            depth,
            |_index, case, _item_depth| async move {
                // `run_eval` invokes this only when a subject is present; run it once per case with the
                // case bound as `${{ case }}`, at the eval task's own recursion depth.
                let case_scope = Scope {
                    case: Some(&case),
                    ..*scope
                };
                match subject {
                    Some(subject) => {
                        self.run_inner_task(
                            id,
                            subject,
                            subject_name,
                            &case_scope,
                            ports,
                            scheduler,
                            masker,
                            resolved_secrets,
                            ctx_env,
                            depth,
                        )
                        .await
                    }
                    None => Ok(Value::Null),
                }
            },
        )
        .await?;
        // `eval.case.finish` carries no duration, so it is emitted once per scored case after the
        // scorecard is built — this also covers a subject-less eval, whose callback never runs.
        let cases = scorecard
            .get("cases")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        for index in 0..cases {
            emit_event(
                ports,
                masker,
                resolved_secrets,
                Event::EvalCaseFinish {
                    name: name.to_string(),
                    index: index as u32,
                },
            )
            .await?;
        }
        Ok(scorecard)
    }

    /// Run one `map`/`eval` inner task (the element task, or the eval `subject`) under `scope`, at
    /// recursion `depth`, returning its normalised output value.
    ///
    /// A leaf inner task crosses its driven port through [`dispatch_task`]; a `flow` inner task recurses
    /// through [`run_subflow`] against a fresh, secrets-seeded masker (the `Fn` fan-out callback cannot
    /// borrow the run masker mutably, so a sub-flow masks its own newly-resolved secrets locally and its
    /// output is scrubbed before merge); a nested `map`/`eval` inner task recurses through this unit's
    /// own fan-out seams.
    ///
    /// Returns a type-erased boxed future: the fan-out seams (`run_map_task` → `run_map` → this
    /// callback → nested fan-out) are mutually recursive `async fn`s, so erasing this one future's type
    /// breaks the otherwise-infinite opaque-type cycle — the same discipline [`run_tasks`] uses for
    /// `flow` recursion.
    #[allow(clippy::too_many_arguments)] // the inner-task collaborators; threaded from the fan-out
    fn run_inner_task<'a, S: Scheduler>(
        &'a self,
        id: &'a RunId,
        inner: &'a Task,
        inner_name: &'a str,
        scope: &'a Scope<'a>,
        ports: Ports<'a>,
        scheduler: &'a S,
        masker: &'a Masker,
        resolved_secrets: &'a [String],
        ctx_env: &'a EnvMap,
        depth: u32,
    ) -> Pin<Box<dyn Future<Output = Result<Value, RunError>> + Send + 'a>> {
        Box::pin(async move {
            match dispatch_task(inner, inner_name, scope, ports, ctx_env, depth).await? {
                Dispatch::Leaf(adapter) => Ok(normalize_output(adapter)),
                Dispatch::Flow(fw) => {
                    // Seed a fresh masker with the run's already-resolved secrets so the sub-flow's
                    // events mask correctly; scrub its output with that same masker before it is merged,
                    // so a secret the sub-flow resolves never leaks into the parent state (the parent
                    // masker re-scrubs idempotently at run end).
                    let mut sub_masker = Masker::new();
                    for secret in resolved_secrets {
                        sub_masker.register(secret.clone());
                    }
                    let mut sub_secrets = resolved_secrets.to_vec();
                    let state = self
                        .run_subflow(
                            id,
                            fw,
                            scope,
                            ports,
                            scheduler,
                            &mut sub_masker,
                            &mut sub_secrets,
                            depth,
                        )
                        .await?;
                    Ok(sub_masker.redact_value(&state).into_inner().into_owned())
                }
                Dispatch::Map(mw) => {
                    self.run_map_task(
                        id,
                        mw,
                        inner_name,
                        scope,
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        ctx_env,
                        depth,
                    )
                    .await
                }
                Dispatch::Eval(ew) => {
                    self.run_eval_task(
                        id,
                        ew,
                        inner_name,
                        scope,
                        ports,
                        scheduler,
                        masker,
                        resolved_secrets,
                        ctx_env,
                        depth,
                    )
                    .await
                }
            }
        })
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

/// Classify a step error as a cancellation, by its [`ErrorCategory`]: `Timeout` → [`CancelReason::Timeout`],
/// `Interrupt` → [`CancelReason::Interrupt`], anything else `None`. Only [`CancelToken::guard`] (and a
/// cancellation propagated up from a sub-flow) produces those two categories — a per-task `timeout` is
/// a `RunFailure` (`task_timeout`), so a genuine task failure is never mis-read as a cancellation.
fn cancel_reason_of(err: &RunError) -> Option<CancelReason> {
    match err.category {
        crate::error::ErrorCategory::Timeout => Some(CancelReason::Timeout),
        crate::error::ErrorCategory::Interrupt => Some(CancelReason::Interrupt),
        _ => None,
    }
}

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

// ---------------------------------------------------------------------------------------------
// Task slicing and matrix lowering — the pure `tmx run` flag transforms over a `ResolvedFlow`
// (07 §`tmx run` run flags, §Matrix sugar). Both are pure: they rewrite the resolved task list /
// produce the matrix cross-product; the CLI applies them before handing the flow to the runner.
// ---------------------------------------------------------------------------------------------

/// The task-slicing selection built from `--from`/`--until`/`--only`/`--skip` (07 §`tmx run`).
///
/// Slicing narrows the sequential task list while preserving source order; it pairs with `--state-in`
/// so a later task still reads a prior task's state via `${{ tasks.NAME.field }}`. Every named task
/// must exist in the flow — an unknown name is a typed `unknown_task` error, never a silent no-op.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskSlice {
    /// Keep only tasks at or after this task (inclusive). `None` starts at the first task.
    pub from: Option<String>,
    /// Keep only tasks at or before this task (inclusive). `None` ends at the last task.
    pub until: Option<String>,
    /// When non-empty, keep only tasks whose name is listed here.
    pub only: Vec<String>,
    /// Drop every task whose name is listed here.
    pub skip: Vec<String>,
}

impl TaskSlice {
    /// Whether this selection narrows anything (any of the four fields is set).
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.from.is_none() && self.until.is_none() && self.only.is_empty() && self.skip.is_empty()
    }
}

/// Apply a [`TaskSlice`] to a [`ResolvedFlow`], returning a flow whose `tasks` are the selected
/// subset in source order (07 §Slicing pairs with `--state-in`).
///
/// The transforms compose in a fixed order: the `from`/`until` range first (a contiguous window),
/// then `only` (intersection), then `skip` (removal). Every named task must exist; an unknown name in
/// any of the four fields is a typed `unknown_task` [`RunError::validation`].
///
/// # Errors
///
/// Returns `unknown_task` when `from`/`until`/`only`/`skip` names a task the flow does not declare.
pub fn slice_tasks(flow: ResolvedFlow, slice: &TaskSlice) -> Result<ResolvedFlow, RunError> {
    if slice.is_noop() {
        return Ok(flow);
    }
    let names: std::collections::HashSet<&str> = flow
        .tasks
        .iter()
        .filter_map(|t| t.name.as_deref())
        .collect();
    // Every referenced task must exist — an unknown name is the negative space of "slice by name".
    for referenced in slice
        .from
        .iter()
        .chain(slice.until.iter())
        .chain(slice.only.iter())
        .chain(slice.skip.iter())
    {
        if !names.contains(referenced.as_str()) {
            return Err(RunError::validation(
                "unknown_task",
                format!("--only/--skip/--from/--until names unknown task {referenced:?}"),
            ));
        }
    }

    let from_index = match &slice.from {
        Some(name) => flow
            .tasks
            .iter()
            .position(|t| t.name.as_deref() == Some(name)),
        None => Some(0),
    };
    let until_index = match &slice.until {
        Some(name) => flow
            .tasks
            .iter()
            .position(|t| t.name.as_deref() == Some(name)),
        None => Some(flow.tasks.len().saturating_sub(1)),
    };
    // Both indices resolve (the existence check above guarantees it); an empty flow with no from/until
    // simply yields no tasks.
    let (lo, hi) = match (from_index, until_index) {
        (Some(lo), Some(hi)) if flow.tasks.is_empty() => (lo, hi),
        (Some(lo), Some(hi)) => (lo, hi),
        _ => (1, 0), // an empty selection (lo > hi)
    };

    let only: std::collections::HashSet<&str> = slice.only.iter().map(String::as_str).collect();
    let skip: std::collections::HashSet<&str> = slice.skip.iter().map(String::as_str).collect();

    let ResolvedFlow {
        name,
        description,
        version,
        environment,
        context,
        inputs,
        tasks,
    } = flow;
    let selected: Vec<Task> = tasks
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index >= lo && *index <= hi)
        .map(|(_, task)| task)
        .filter(|task| {
            let name = task.name.as_deref().unwrap_or("");
            (only.is_empty() || only.contains(name)) && !skip.contains(name)
        })
        .collect();

    Ok(ResolvedFlow {
        name,
        description,
        version,
        environment,
        context,
        inputs,
        tasks: selected,
    })
}

/// Whether `flow` declares an authored `map` task at the top level — the guard for "an authored `map`
/// wins over `--matrix`" (07 §Matrix sugar): the CLI never rewrites or wraps an explicit `map`.
#[must_use]
pub fn flow_has_map(flow: &ResolvedFlow) -> bool {
    flow.tasks
        .iter()
        .any(|task| matches!(task.with, TaskWith::Map(_)))
}

/// Lower `--matrix` axes into the bounded cross-product of combinations (07 §Matrix sugar).
///
/// Each axis is `key → [v1, v2, …]`; the result is one JSON object per combination binding every
/// `${{ matrix.<key> }}`, in a deterministic order (axes in declaration order, each varying fastest to
/// slowest right-to-left). The cross-product width is bounded by [`FANOUT_WIDTH_MAX`] — an over-wide
/// matrix is a typed `fanout_too_wide` error, not a silent truncation. An axis with an empty value
/// list collapses the product to zero combinations (there is nothing to bind).
///
/// # Errors
///
/// Returns `fanout_too_wide` when the cross-product exceeds [`FANOUT_WIDTH_MAX`] combinations.
pub fn matrix_combinations(axes: &IndexMap<String, Vec<Value>>) -> Result<Vec<Value>, RunError> {
    if axes.is_empty() {
        return Ok(Vec::new());
    }
    // The product width, computed with saturating arithmetic so an enormous matrix cannot overflow
    // before the bound check rejects it.
    let mut width: u64 = 1;
    for values in axes.values() {
        width = width.saturating_mul(values.len() as u64);
    }
    if width > u64::from(FANOUT_WIDTH_MAX) {
        return Err(RunError::run_failure(
            "fanout_too_wide",
            format!(
                "--matrix cross-product is {width} combinations, exceeding the {FANOUT_WIDTH_MAX} fan-out width limit"
            ),
        ));
    }
    // A zero-width product (an empty axis) yields no combinations.
    if width == 0 {
        return Ok(Vec::new());
    }

    // Build the cross-product by folding each axis over the running set of partial combinations.
    let mut combos: Vec<serde_json::Map<String, Value>> = vec![serde_json::Map::new()];
    for (key, values) in axes {
        let mut next: Vec<serde_json::Map<String, Value>> =
            Vec::with_capacity(combos.len() * values.len());
        for base in &combos {
            for value in values {
                let mut extended = base.clone();
                extended.insert(key.clone(), value.clone());
                next.push(extended);
            }
        }
        combos = next;
    }
    // Paired assertion: the built count matches the computed product width.
    assert_eq!(
        combos.len() as u64,
        width,
        "the built combination count equals the computed cross-product width"
    );
    let out: Vec<Value> = combos.into_iter().map(Value::Object).collect();
    assert!(
        out.len() as u64 <= u64::from(FANOUT_WIDTH_MAX),
        "the matrix cross-product stays within FANOUT_WIDTH_MAX"
    );
    Ok(out)
}

#[cfg(test)]
mod slice_matrix_tests {
    use super::*;
    use serde_json::json;
    use tmx_schema::task::{ExecWith, TaskWith};

    /// A minimal named `exec` task fixture for slicing tests.
    fn exec_task(name: &str) -> Task {
        Task {
            kind: None,
            name: Some(name.to_string()),
            description: None,
            if_condition: None,
            secrets: None,
            context: None,
            context_strategy: None,
            context_precedence: None,
            output: None,
            produces: None,
            continue_on_error: None,
            with: TaskWith::Exec(ExecWith {
                command: "noop".to_string(),
                args: None,
                shell: None,
                cwd: None,
                env: None,
                timeout: None,
            }),
        }
    }

    fn flow_of(names: &[&str]) -> ResolvedFlow {
        ResolvedFlow {
            name: Some("f".to_string()),
            description: None,
            version: None,
            environment: None,
            context: None,
            inputs: IndexMap::new(),
            tasks: names.iter().map(|n| exec_task(n)).collect(),
        }
    }

    fn task_names(flow: &ResolvedFlow) -> Vec<String> {
        flow.tasks
            .iter()
            .map(|t| t.name.clone().unwrap_or_default())
            .collect()
    }

    #[test]
    fn from_until_selects_an_inclusive_contiguous_window() {
        // `--from b --until d` keeps b,c,d — the inclusive range in source order.
        let flow = flow_of(&["a", "b", "c", "d", "e"]);
        let slice = TaskSlice {
            from: Some("b".to_string()),
            until: Some("d".to_string()),
            ..TaskSlice::default()
        };
        let sliced = slice_tasks(flow, &slice).expect("the window resolves");
        assert_eq!(task_names(&sliced), vec!["b", "c", "d"], "inclusive b..=d");
        assert!(!slice.is_noop(), "a set slice is not a no-op");
    }

    #[test]
    fn only_and_skip_filter_within_the_window() {
        // `--only a,c,e --skip c` keeps a,e (only intersects, skip removes), order preserved.
        let flow = flow_of(&["a", "b", "c", "d", "e"]);
        let slice = TaskSlice {
            only: vec!["a".to_string(), "c".to_string(), "e".to_string()],
            skip: vec!["c".to_string()],
            ..TaskSlice::default()
        };
        let sliced = slice_tasks(flow, &slice).expect("only/skip resolve");
        assert_eq!(task_names(&sliced), vec!["a", "e"], "only∩ minus skip");
    }

    #[test]
    fn a_noop_slice_returns_the_flow_unchanged() {
        // The empty selection is an identity — every task survives, in order.
        let flow = flow_of(&["a", "b"]);
        let slice = TaskSlice::default();
        assert!(slice.is_noop(), "an empty selection is a no-op");
        let sliced = slice_tasks(flow, &slice).expect("a no-op slice is Ok");
        assert_eq!(task_names(&sliced), vec!["a", "b"], "identity");
    }

    #[test]
    fn an_unknown_task_name_is_a_typed_error() {
        // Negative space: naming a task the flow does not declare is `unknown_task`, not a silent skip.
        let flow = flow_of(&["a", "b"]);
        let slice = TaskSlice {
            from: Some("ghost".to_string()),
            ..TaskSlice::default()
        };
        let err = slice_tasks(flow, &slice).expect_err("an unknown task is rejected");
        assert_eq!(err.code, "unknown_task", "the unknown-task code");
        assert!(
            err.message.contains("ghost"),
            "the message names the missing task, got {:?}",
            err.message
        );
    }

    #[test]
    fn matrix_two_axes_yield_the_full_cross_product() {
        // `a=1,2` × `b=x,y` → four combinations, each binding both keys.
        let mut axes: IndexMap<String, Vec<Value>> = IndexMap::new();
        axes.insert("a".to_string(), vec![json!(1), json!(2)]);
        axes.insert("b".to_string(), vec![json!("x"), json!("y")]);
        let combos = matrix_combinations(&axes).expect("the cross-product builds");
        assert_eq!(combos.len(), 4, "2×2 is a four-way cross-product");
        assert_eq!(
            combos[0],
            json!({ "a": 1, "b": "x" }),
            "the first combination binds both axes"
        );
        assert_eq!(
            combos[3],
            json!({ "a": 2, "b": "y" }),
            "the last combination is the far corner of the product"
        );
    }

    #[test]
    fn matrix_no_axes_and_an_empty_axis_yield_no_combinations() {
        // No axes → no combinations (a matrix-free run); an empty axis collapses the product to zero.
        assert!(
            matrix_combinations(&IndexMap::new())
                .expect("no axes is Ok")
                .is_empty(),
            "no axes yields no combinations"
        );
        let mut axes: IndexMap<String, Vec<Value>> = IndexMap::new();
        axes.insert("a".to_string(), vec![json!(1)]);
        axes.insert("b".to_string(), Vec::new());
        assert!(
            matrix_combinations(&axes)
                .expect("an empty axis is Ok")
                .is_empty(),
            "an empty axis collapses the cross-product to zero"
        );
    }

    #[test]
    fn an_over_width_matrix_is_fanout_too_wide() {
        // Negative space: a cross-product beyond FANOUT_WIDTH_MAX is rejected, not truncated.
        let big = (0..=FANOUT_WIDTH_MAX).map(|i| json!(i)).collect::<Vec<_>>();
        let mut axes: IndexMap<String, Vec<Value>> = IndexMap::new();
        axes.insert("a".to_string(), big);
        axes.insert("b".to_string(), vec![json!(1), json!(2)]);
        let err = matrix_combinations(&axes).expect_err("an over-width matrix is rejected");
        assert_eq!(err.code, "fanout_too_wide", "the width error code");
    }

    #[test]
    fn flow_has_map_detects_an_authored_map() {
        // `flow_has_map` is the authored-`map`-wins guard for `--matrix`.
        let plain = flow_of(&["a"]);
        assert!(!flow_has_map(&plain), "a map-free flow has no authored map");

        let mut mapped = flow_of(&["fan"]);
        mapped.tasks[0].with = TaskWith::Map(
            serde_json::from_value(json!({
                "items": ["a"],
                "task": { "type": "exec", "with": { "command": "noop" } },
            }))
            .expect("valid MapWith"),
        );
        assert!(
            flow_has_map(&mapped),
            "a flow with a map task is detected so --matrix is ignored"
        );
    }
}
