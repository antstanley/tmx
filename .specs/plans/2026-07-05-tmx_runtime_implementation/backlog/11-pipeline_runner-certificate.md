# Done Certificate — Task 11: PipelineRunner (the sequential task loop)

**Task:** [11-pipeline_runner.md](11-pipeline_runner.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

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
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the loop and recursion
    bounds are the named `tmx-schema::limits` constants `TASKS_PER_FLOW_MAX`/`FLOW_DEPTH_MAX`, not
    magic numbers; run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio`
    expecting no match) confirming `tmx-core` stays free of an async-runtime/I/O edge even though the
    runner is `async`.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the runner integration test over the fakes and confirm the recorded event stream and masked final state match the expected golden values (Reviewable).**
  - *Claim:* a reviewer can run the runner integration test over the fakes and observe the recorded
    event stream and the masked final state match the expected golden values.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core runner` and read the summary; confirm
    the integration test asserts both the ordered event stream and the masked final state against
    golden values (fixed `Clock`/`IdGenerator` fakes make this deterministic) with zero failures.
  - *Status:* ☐ unverified

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
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
