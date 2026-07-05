# Task 18 — Scheduler port and `map` fan-out

**Plan:** [plan.md](../plan.md) · **Certificate:** [18-scheduler_and_map-certificate.md](18-scheduler_and_map-certificate.md)

**Implements:** [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §The Scheduler, §`map` — bounded fan-out; [04-execution-engine.md](../../../04-execution-engine.md) §Decisions (side-effect ordering under concurrency)
**Depends on:** 11
**Produces:** the `map` orchestration in the core over the `Scheduler` port, plus the production `TokioScheduler` adapter — bounded concurrent fan-out that always collects in item order
**Pointers:** `crates/tmx-core/src/fanout.rs` (new, `map`), `crates/tmx-adapters/src/scheduler.rs` (new, `TokioScheduler`)

## Steps

- [ ] Implement `map`: resolve `items` to an array and assert `len <= FANOUT_WIDTH_MAX` (`fanout_too_wide` when an expression yields an over-limit array), build `n` child scopes binding the element under `as` (with `.index`), and run the inner task through `Scheduler.run_indexed`.
- [ ] Collect into an ordered `Vec` asserting output length equals input length on both the producing and consuming side, and apply the element error policy (`continueOnError` records the error in the slot; else abort the `map`).
- [ ] Implement the bounded `TokioScheduler` (alongside the minimal serial production scheduler task 17 already wired for the default `concurrency: 1`) bounding in-flight work with a semaphore sized by the task `concurrency` (capped by `--concurrency` and `CONCURRENCY_MAX`), asserting `concurrency >= 1`, in-flight `<= concurrency`, and a returned vector of length `n` in index order.
- [ ] Increment the recursion `depth` when the inner task is a `flow`, and merge `state[name] = [ … ]` in item order.

## Definition of done

- [ ] A `map` over a collection runs its inner task per element and merges an array in item order regardless of completion order, with `concurrency` honoured up to the cap.
- [ ] An over-`FANOUT_WIDTH_MAX` resolved collection returns `fanout_too_wide` and an over-`CONCURRENCY_MAX` request is rejected (negative space), and the output-length assertion holds on both sides.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run a `map` with `concurrency > 1` under the `TokioScheduler` and confirm item-ordered output, then run it under the `SerialScheduler` for identical results.
