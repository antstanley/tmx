# Task 33 — Wire map/eval task dispatch into the runner

**Plan:** [plan.md](../plan.md) · **Certificate:** [33-wire_map_eval_dispatch-certificate.md](33-wire_map_eval_dispatch-certificate.md)

> **Post-hoc gap-closure task.** The 32-task plan built `run_map`/`run_eval` (tasks 18/19) and their event rendering (task 26) and conformance coverage (task 32), but no task wired the two control-flow task types into the runner's per-task dispatch — `dispatch_task` (`crates/tmx-core/src/dispatch.rs`) returns `task_type_unsupported` for `Map`/`Eval`, so a Flow containing a `map` or `eval` task fails at runtime. Four verifiers (18, 26, 30, 32) flagged this as deferred. This task closes it so both spec task types execute end-to-end.

**Implements:** [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §The `map` task, §The `eval` task, §Scheduling; [04-execution-engine.md](../../../04-execution-engine.md) §Task dispatch
**Depends on:** 18, 19, 26, 30, 32 (all done)
**Produces:** a Flow with a `map` task and a Flow with an `eval` task both run end-to-end through `RunFlow`/`tmx run` — `map` collects a per-item output array in item order under bounded concurrency; `eval` emits a `Scorecard`; both emit their `map.item.finish`/`eval.case.finish` events; every existing test stays green and the purity gate stays green.
**Pointers:** `crates/tmx-core/src/dispatch.rs:116` (the `TaskWith::Map(_) | TaskWith::Eval(_)` rejection), `crates/tmx-core/src/runner.rs` (the `Ports` bundle ~L15 with its "Scheduler deliberately absent" note, the dispatch loop at ~L685, and `run_subflow` for the recursion pattern), `crates/tmx-core/src/fanout.rs:55` (`run_map`) and `:239` (`run_eval`), `crates/tmx-cli/src/compose.rs` (inject the concrete `TokioScheduler`), `crates/tmx-adapters/src/scheduler.rs` (`TokioScheduler`), `crates/tmx-testkit` (`SerialScheduler` for tests).

## Steps

- [x] Give the sequential runner access to a `Scheduler` port (inject it — e.g. add it to the `Ports` bundle or make the run generic over `S: Scheduler`), keeping `tmx-core` pure: the scheduler is the existing driven port, `TokioScheduler` stays in `tmx-adapters`, `SerialScheduler` in `tmx-testkit`. The purity gate must stay green (no tokio/futures edge into core).
- [x] Route `TaskWith::Map` to `run_map` and `TaskWith::Eval` to `run_eval` from the runner's per-task dispatch, passing the injected scheduler, the run's `--concurrency` cap, the current `flow`-recursion `depth`, and a `run_item`/per-case callback that dispatches the inner task exactly as `run_subflow`/`dispatch_task` do (incrementing depth when the inner task is a `flow`, bounded by `FLOW_DEPTH_MAX`). Remove the `task_type_unsupported` rejection for Map/Eval (keep it as the backstop for any genuinely unknown type).
- [x] Emit the `map.item.finish` and `eval.case.finish` events at the right transitions (they are already rendered by every sink from task 26), plus the surrounding `task.start`/`task.finish` for the map/eval task itself.
- [x] Preserve every existing guard and typed error from `run_map`/`run_eval` (`fanout_too_wide`, `concurrency_too_high`, `map_items_not_array`, `flow_depth_exceeded`, eval threshold gating) — they must surface unchanged through the runner.
- [x] Add an end-to-end test that runs a `map` Flow and an `eval` Flow through `RunFlow` over the fakes (and, where the DoD calls for it, through the real `tmx run` binary), asserting the collected array is in item order and the `Scorecard` is produced.

## Definition of done

- [x] A Flow whose task list contains a `map` task runs end-to-end via `RunFlow` (and `tmx run`): the map fans out over its `items` under the bounded scheduler, binds each element as `as`, and collects the per-item outputs into an array **in item order**; `continueOnError` and the concurrency cap behave per spec.
- [x] A Flow containing an `eval` task runs end-to-end and produces a `Scorecard`; a missed `threshold` fails the run.
- [x] `dispatch_task` no longer returns `task_type_unsupported` for `Map`/`Eval`; the negative-space backstop remains for a genuinely unknown type.
- [x] The full suite stays green — `cargo nextest run` (all prior tests plus the new end-to-end map/eval tests), `cargo clippy --all-targets --all-features -D warnings`, `cargo fmt --all --check`, and `scripts/purity.sh` (core stays I/O/async-free; the scheduler is injected as a port). Meets the repo definition of done.
- [x] Reviewable: from the shell, `tmx run` a Flow with a `map` task and observe the collected per-region/per-item output array in the final state, and `tmx run` a Flow with an `eval` task and observe the `Scorecard` — neither returns `task_type_unsupported`.
