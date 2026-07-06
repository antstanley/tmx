//! Lifecycle hooks — `create` / `change` / `destroy` / `error`, one level deep.
//!
//! A [`HookRunner`] fires a Flow's `context.hooks` bodies through the **same** [`PipelineRunner`]
//! ([04 §Lifecycle hooks](../../../.specs/04-execution-engine.md)), so a hook inherits the full task
//! model (it may `exec`, `fetch`, even `map`). Two rules keep hooks safe and terminating:
//!
//! - **One level deep.** A hook body runs on a [`PipelineRunner::for_hook_body`] runner, which walks
//!   its tasks without firing any lifecycle hook of its own — so a `change` hook that mutates state
//!   does not re-trigger `change` (no hook-storm). [`HookRunner::fire`] also carries the asserted
//!   backstop: it refuses to fire while already inside a hook.
//! - **`change` fires once per state-changing task**, and only when the merge actually changed the
//!   state — the runner's task loop, not this unit, decides *when* to call `fire`.
//!
//! This unit is pure in the same sense as the runner: it awaits driven ports but holds no I/O of its
//! own, so the crate stays inside the purity boundary.

use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::context::{Context, Hook};
use tmx_schema::limits::HOOK_TASKS_MAX;

use crate::error::RunError;
use crate::mask::Masker;
use crate::model::{Event, Milliseconds, ResolvedFlow, RunId, TaskStatus};
use crate::ports::driven::Scheduler;
use crate::resolve::desugar_tasks;
use crate::runner::{PipelineRunner, Ports, RunConfig, emit_event};

/// Which Pipeline-lifecycle transition a hook fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    /// Fires once on entry to `running`.
    Create,
    /// Fires once per state-changing task, after its merge changed the state.
    Change,
    /// Fires on every terminal status — the `finally` of the lifecycle.
    Destroy,
    /// Fires when a task aborts the Pipeline (not under `continueOnError`).
    Error,
}

impl HookKind {
    /// Every kind, in lifecycle order — exercises the exhaustiveness of the `match`es over the kind.
    pub const ALL: [HookKind; 4] = [
        HookKind::Create,
        HookKind::Change,
        HookKind::Destroy,
        HookKind::Error,
    ];

    /// The stable lower-case token for this kind (`create` / `change` / `destroy` / `error`), used as
    /// the `hook.start` / `hook.finish` event name. Exhaustive `match`, no wildcard.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            HookKind::Create => "create",
            HookKind::Change => "change",
            HookKind::Destroy => "destroy",
            HookKind::Error => "error",
        }
    }
}

/// Fires a Flow's lifecycle-hook bodies through the same [`PipelineRunner`], one level deep.
///
/// Constructed once per [`PipelineRunner::run`] from the Flow's resolved `context`. It borrows the
/// context (no clone of the hook bodies until one actually fires), so it is a cheap handle threaded
/// through the run. `in_hook` records whether the *surrounding* run is itself a hook body: when set,
/// [`HookRunner::fire`] is a no-op, the structural half of the no-hook-inside-a-hook guarantee.
pub struct HookRunner<'a> {
    /// The Flow's `context`, when it declares one (the source of both the hook bodies and the
    /// env/secrets a hook body inherits).
    context: Option<&'a Context>,
    /// The engine flags a hook body's runner inherits (state cap, `continueOnError`, `produces`).
    config: RunConfig,
    /// Whether the surrounding run is itself a hook body — if so, no lifecycle hook fires.
    in_hook: bool,
    /// The run's `--matrix` combination, so a hook body reads the same `${{ matrix.<key> }}` as the
    /// tasks that triggered it. An empty object for a matrix-free run.
    matrix: Value,
}

impl<'a> HookRunner<'a> {
    /// Wire a hook runner over a Flow's `context`, the engine `config`, whether the surrounding run is
    /// already a hook body (`in_hook`), and the run's `matrix` binding (propagated to hook bodies).
    #[must_use]
    pub fn new(
        context: Option<&'a Context>,
        config: RunConfig,
        in_hook: bool,
        matrix: Value,
    ) -> Self {
        Self {
            context,
            config,
            in_hook,
            matrix,
        }
    }

    /// The declared hook body for `kind`, if any.
    fn body_for(&self, kind: HookKind) -> Option<&'a Hook> {
        let hooks = self.context?.hooks.as_ref()?;
        match kind {
            HookKind::Create => hooks.create.as_ref(),
            HookKind::Change => hooks.change.as_ref(),
            HookKind::Destroy => hooks.destroy.as_ref(),
            HookKind::Error => hooks.error.as_ref(),
        }
    }

    /// Fire the `kind` hook, if one is declared and we are not already inside a hook.
    ///
    /// Returns `Ok(true)` when a body actually ran (a `hook.start`/`hook.finish` pair was emitted),
    /// `Ok(false)` when there was nothing to fire (no such hook, or suppressed because the surrounding
    /// run is a hook body). A body whose own tasks fail is **not** an error here — the failure is
    /// reflected in the `hook.finish` status and does not mask the Pipeline's terminal status. `Err` is
    /// reserved for a structural fault: an over-[`HOOK_TASKS_MAX`] body, an unsupported hook reference,
    /// or an output-port write failure.
    #[allow(clippy::too_many_arguments)] // mirrors the runner's collaborators; a hook body is a full run
    pub async fn fire<S: Scheduler>(
        &self,
        kind: HookKind,
        id: &RunId,
        inputs: &Value,
        ports: Ports<'_>,
        scheduler: &S,
        masker: &mut Masker,
        resolved_secrets: &mut Vec<String>,
        depth: u32,
    ) -> Result<bool, RunError> {
        let Some(hook) = self.body_for(kind) else {
            return Ok(false);
        };
        // One level deep: a lifecycle hook must never fire inside another hook (no hook-storm). A hook
        // body runs on a `for_hook_body` runner that never reaches this call, so in normal operation
        // this holds structurally; the assertion is the load-bearing backstop against a misuse.
        assert!(
            !self.in_hook,
            "a lifecycle hook must not fire inside another hook (one level deep)"
        );

        let body = self.body_flow(hook, &kind)?;
        // The hook body task count is bounded by HOOK_TASKS_MAX (a named units-less limit); an
        // over-limit body is rejected before it runs.
        if body.tasks.len() as u64 > u64::from(HOOK_TASKS_MAX) {
            return Err(RunError::validation(
                "too_many_hook_tasks",
                format!(
                    "a {} hook may hold at most {HOOK_TASKS_MAX} tasks",
                    kind.as_str()
                ),
            ));
        }

        let name = kind.as_str().to_string();
        let start_ms = ports.clock.now_ms();
        emit_event(
            ports,
            masker,
            resolved_secrets,
            Event::HookStart { name: name.clone() },
        )
        .await?;

        // Run the body one level deep: a `for_hook_body` runner walks its tasks without firing hooks,
        // and `run_tasks` is given `None`, so no `change` fires per hook task either.
        let body_runner =
            PipelineRunner::for_hook_body(self.config).with_matrix(self.matrix.clone());
        let outcome = body_runner
            .run_tasks(
                id,
                &body,
                inputs,
                ports,
                scheduler,
                masker,
                resolved_secrets,
                None,
                depth,
                None,
            )
            .await?;

        // A hook body is a full run: like any run, it must leave a terminal status. This mirrors the
        // runner's own end-of-run invariant, one level down.
        assert!(
            outcome.pipeline.status.is_terminal(),
            "a hook body run must reach a terminal status"
        );
        let status = if outcome.failure.is_some() {
            TaskStatus::Error
        } else {
            TaskStatus::Ok
        };
        let ms = Milliseconds(ports.clock.now_ms().0.saturating_sub(start_ms.0));
        emit_event(
            ports,
            masker,
            resolved_secrets,
            Event::HookFinish { name, status, ms },
        )
        .await?;
        Ok(true)
    }

    /// Build the [`ResolvedFlow`] a hook body runs as. An inline task set inherits the Flow's
    /// `context` (its env/secrets), so the hook runs with the same environment as the tasks that
    /// triggered it. A *referenced* hook body (`use` / a path/name string) is resolved by the loading
    /// unit, not the runner — an explicit `unsupported_reference` here, so nothing is silently dropped.
    fn body_flow(&self, hook: &Hook, kind: &HookKind) -> Result<ResolvedFlow, RunError> {
        match hook {
            Hook::Tasks(tasks) => {
                let tasks = desugar_tasks(tasks.clone())?;
                Ok(ResolvedFlow {
                    name: Some(format!("{}-hook", kind.as_str())),
                    description: None,
                    version: None,
                    environment: None,
                    context: self.context.cloned(),
                    inputs: IndexMap::new(),
                    tasks,
                })
            }
            Hook::Reference(_) | Hook::Use(_) => Err(RunError::resolution(
                "unsupported_reference",
                "a referenced hook body is resolved by the loading unit, not the runner",
            )),
        }
    }
}
