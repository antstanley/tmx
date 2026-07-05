# Done Certificate — Task 11: PipelineRunner (the sequential task loop)

**Task:** [11-pipeline_runner.md](11-pipeline_runner.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

> This certificate is a verification protocol for Task 11. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 11) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion. O4 is the
Reviewable item; record DONE only when O1…O4 are all SATISFIED.

## Premises

- **P1 — Goal.** Produce `PipelineRunner::run` — the bounded sequential loop that turns a
  `ResolvedFlow` into a final Pipeline state through the fakes, plus the `TaskDispatcher` seam wiring
  `assert` (pure) and routing side-effecting types to their ports.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This unit composes the four Task M2 pure services — the interpolator (07), the
  MatcherEngine (08), the Masker (09), and state merge (10) — plus the Task 06 fake port bundle. Each
  composed service's own unit/property tests must keep passing while the runner integrates them; the
  runner adds behavior, it does not alter those units.

## Obligations

- **O1 — `RunFlow` over the fake bundle runs a multi-task `assert`/(fake) `exec` flow, emits the canonical event stream in order, returns the masked final state, and honours the `continueOnError`-vs-abort error policy.**
  - *Claim:* the `RunFlow` use case over the Task 06 fakes runs a multi-task flow mixing `assert` and
    fake `exec` tasks, emits `run.start`/`task.start`/`task.finish`/`task.skip`/`task.error`/
    `run.finish` in order, and returns the masked final state; a failing non-`continueOnError` task
    stops the loop while a `continueOnError` task records its error and continues.
  - *Evidence to collect:* read `crates/tmx-core/src/runner.rs`, `crates/tmx-core/src/dispatch.rs`,
    and the `RunFlow` impl in `crates/tmx-core/src/usecases.rs`; run
    `cargo nextest run -p tmx-core runner` and confirm the multi-task integration test passes, the
    recorded event order matches the canonical stream, and both the abort and the `continueOnError`
    branches are exercised with the expected stop/continue behavior.
  - *Checks:* resolve the dispatch of an `assert` task to the pure `MatcherEngine` call, not a driven
    port; resolve a fake `exec` task to the `ProcessRunner` port; confirm both are reached from the
    single `TaskDispatcher` seam.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core runner` → 8/8 pass. Traced
    `runner_runs_multi_task_flow_emits_ordered_stream_and_masked_state`: `exec` (`b"built-ok"` →
    `{"message":"built-ok"}`) then `assert` reading `${{ tasks.build.message }}`; emitted tags are
    exactly `run.start, task.start, task.finish, task.start, task.finish, run.finish`; final state
    golden `{"build":{"message":"built-ok"},"check":{"passed":true,"assertions":1}}`. `assert` routes
    to the pure `MatcherEngine` (dispatch.rs `TaskWith::Assert → run_assert`, no port); `exec` routes
    to `ports.process.run` — both from the single `dispatch_task` seam. Abort branch stops the loop (1
    result, Failed); continue branch records the error and runs the next task (2 results, Ok).

- **O2 — The load-bearing invariants assert, and a too-deep `flow` nest returns `flow_depth_exceeded`.**
  - *Claim:* the runner asserts unique task names, in-range task index, object-typed state, `depth <=
    FLOW_DEPTH_MAX`, and the Masker registry populated before any emission; a `flow` nest that would
    exceed the depth bound returns `ResolutionError` `flow_depth_exceeded` (negative space) before
    recursing.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core runner` and confirm the invariant
    assertions are present and covered (unique names, index range, object state, depth bound, Masker
    populated before output), and the too-deep-nest test returns `flow_depth_exceeded`; read the
    dispatcher and confirm the `type`→port match asserts exhaustiveness over the closed enum with no
    fallthrough.
  - *Checks:* trace a `flow` task at `depth == FLOW_DEPTH_MAX` and confirm the guard asserts
    `depth + 1 <= FLOW_DEPTH_MAX` and yields `flow_depth_exceeded` before any recursion into
    `PipelineRunner::run`; trace the secret-resolution path and confirm a name absent from
    `task.secrets` is never resolved and every resolved secret is registered with the Masker before
    dispatch.
  - *Status:* ☑ SATISFIED — Invariants asserted in release (`assert!`, not `debug_assert!`): unique
    names (`validate_task_names` + `assert_eq!` backstop), in-range index (`assert!(index < count)`),
    object state (`assert!(builder.as_value().is_object())` each iteration), non-empty merge key,
    depth backstop (`assert!(depth < FLOW_DEPTH_MAX)` ≡ spec `depth + 1 <= FLOW_DEPTH_MAX`), and
    Masker-populated-before-emit (`emit_event → masker.assert_ready`, a real release `assert!` with
    its own trip tests in mask.rs). Negative space:
    `runner_flow_task_past_the_depth_bound_yields_flow_depth_exceeded` starts at depth 8, the
    dispatch guard `depth >= FLOW_DEPTH_MAX` returns `flow_depth_exceeded` (category Resolution)
    BEFORE any load (reference resolver never consulted; tags `run.start,task.start,task.error,
    run.finish`); `runner_rejects_missing_and_duplicate_task_names` covers the pre-flight name errors.
    Dispatch match is exhaustive over the closed `TaskWith` enum with no `_` wildcard. `resolve_secrets`
    iterates only `task.secrets`; an unrequested name is never touched, and each resolved value is
    `masker.register`ed before dispatch.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the loop and recursion
    bounds are the named `tmx-schema::limits` constants `TASKS_PER_FLOW_MAX`/`FLOW_DEPTH_MAX`, not
    magic numbers; run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio`
    expecting no match) confirming `tmx-core` stays free of an async-runtime/I/O edge even though the
    runner is `async`.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0; `cargo clippy --all-targets
    --all-features -D warnings` exit 0; `cargo nextest run` → 108/108 pass, 0 skipped. Loop bound is
    `TASKS_PER_FLOW_MAX` and recursion bound is `FLOW_DEPTH_MAX`, both `tmx-schema::limits` constants;
    state cap via `STATE_SIZE_MAX_BYTES` in `StateBuilder` — no magic numbers (local
    `MILLISECONDS_PER_*` are units-last conversion factors, not engine dimensions). Purity: `cargo
    tree -p tmx-core -i tokio` / `-i reqwest` → no match; `scripts/purity.sh` green (tmx-testkit is a
    dev-only edge, off the normal-edges gate).

- **O4 — Reviewable: run the runner integration test over the fakes and confirm the recorded event stream and masked final state match the expected golden values (Reviewable).**
  - *Claim:* a reviewer can run the runner integration test over the fakes and observe the recorded
    event stream and the masked final state match the expected golden values.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core runner` and read the summary; confirm
    the integration test asserts both the ordered event stream and the masked final state against
    golden values (fixed `Clock`/`IdGenerator` fakes make this deterministic) with zero failures.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core runner` → 8 passed, 0 failed. The
    integration test asserts BOTH the ordered event-stream tags and the masked final-state golden
    JSON (`FixedClock` + `SeededIdGenerator` make it deterministic); the secret-masking test
    independently confirms redaction on both the emitted `task.finish` payload and the returned final
    state (two boundaries).

## Regression check

- This unit composes Tasks 07/08/09/10. Re-run each composed service's own suite and confirm it still
  passes unchanged: `cargo nextest run -p tmx-core interpolate`, `… matcher`, `… mask`, `… merge`.
  The runner must consume them (interpolate `with`, dispatch `assert` to the MatcherEngine, register/
  redact via the Masker, merge under the resolved key) without forking their behavior.

## Residue

- `contextStrategy`/`contextPrecedence` resolution (`merge`/`replace`, `local`/`inherited`, per
  section) is in the Steps; confirm it is exercised, as DoD item 1 folds it into "runs a multi-task
  flow".
- The optional `produces` conformance hook-in is gated behind `--check-produces` (off by default);
  confirm the seam exists but does not run when the flag is absent.
- The `change` hook trigger point is wired here but its firing semantics are Task 12's obligation;
  confirm this task only exposes the "merge changed the state" signal and does not itself fire hooks.
- Confirm both the emitted event payloads and the returned final state pass through the Masker (two
  independent boundaries), not only the final state.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED. `PipelineRunner::run` is a `TASKS_PER_FLOW_MAX`-bounded sequential
loop that gates on `if`, resolves per-section context and only-requested secrets (masker-registered
before dispatch), interpolates `with`, dispatches through the single exhaustive `TaskDispatcher`
seam (`assert` inline via `MatcherEngine`, six side-effecting types to their ports, `flow`
depth-guarded to a typed `flow_depth_exceeded` before recursion), normalises, merges under
`output ?? name`, and applies the `continueOnError`-vs-abort policy — emitting the canonical
`run.start`/`task.*`/`run.finish` stream, all payloads and the final state redacted by the run
Masker. Load-bearing invariants are release `assert!`s; negative-space (depth ceiling, missing/
duplicate names) is covered by passing tests. Gates independently reproduced green: fmt (0), clippy
`-D warnings` (0), nextest 108/108, runner reviewable 8/8, purity green (no tokio/reqwest edge).
Regression suites interpolate/matcher/mask/merge (56) pass unchanged. The two implementer clippy
fixes (`depth < FLOW_DEPTH_MAX` backstop; panic-closure test helper) are semantically equivalent and
introduce no behavioural change.
