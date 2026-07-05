# Task 11 — PipelineRunner (the sequential task loop)

**Plan:** [plan.md](../plan.md) · **Certificate:** [11-pipeline_runner-certificate.md](11-pipeline_runner-certificate.md)

**Implements:** [04-execution-engine.md](../../../04-execution-engine.md) §Pipeline execution algorithm, §Invariants & assertions, §Secrets & masking (runner side), §Bounded `flow` recursion; [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §TaskDispatcher; [01-domain-model.md](../../../01-domain-model.md) §Pipeline lifecycle
**Depends on:** 06, 07, 08, 09, 10
**Produces:** `PipelineRunner::run` — the bounded sequential loop that turns a `ResolvedFlow` into a final Pipeline state through the fakes, plus the `TaskDispatcher` seam wiring `assert` (pure) and routing side-effecting types to their ports
**Pointers:** `crates/tmx-core/src/runner.rs` (new), `crates/tmx-core/src/dispatch.rs` (new), `crates/tmx-core/src/usecases.rs` (the `RunFlow` impl)

## Steps

- [x] Implement the loop bounded by `TASKS_PER_FLOW_MAX`: `if` gate (falsy → `task.skip`, no `change`), context resolution (`contextStrategy`/`contextPrecedence` per section), `with` interpolation, dispatch, normalize, optional `produces` check hook-in, merge, `change` trigger, and the `continueOnError`-vs-abort error policy.
- [x] Resolve `with` secrets to unmasked values only for names in `task.secrets`, registering each with the Masker before dispatch; never resolve an unrequested secret.
- [x] Implement `TaskDispatcher` mapping `type` → port, asserting exhaustiveness over the closed enum; implement `assert` inline via the MatcherEngine and route `exec`/`run`/`fetch`/`file`/`store`/`chat-completion` to their driven ports (satisfied by testkit fakes here); thread the `depth` parameter and assert `depth + 1 <= FLOW_DEPTH_MAX` before any `flow` recursion.
- [x] Record timing via the `Clock` port, emit `run.start`/`task.start`/`task.finish`/`task.skip`/`task.error`/`run.finish`, and enforce the runner invariants (state always an object, unique task names, index in range, Masker populated before output, terminal status never left).

## Definition of done

- [x] `RunFlow` over the fake bundle runs a multi-task flow of `assert` and (fake) `exec` tasks, emits the canonical event stream in order, and returns the masked final state; a failing non-`continueOnError` task stops the loop and a `continueOnError` task records its error and continues.
- [x] The load-bearing invariants assert (unique names, in-range index, object state, `depth <= FLOW_DEPTH_MAX`, Masker populated before emission), and a too-deep `flow` nest returns `flow_depth_exceeded` (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the runner integration test over the fakes and confirm the recorded event stream and masked final state match the expected golden values.
