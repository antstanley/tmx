# Done Certificate — Task 18: Scheduler port and `map` fan-out

**Task:** [18-scheduler_and_map.md](18-scheduler_and_map.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 18. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 18) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Deliver the `map` orchestration in the core over the `Scheduler` port, plus the production `TokioScheduler` adapter — bounded concurrent fan-out that always collects in item order.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-11 runner's existing multi-task `assert`/`exec` path must keep passing — a map-free flow runs unchanged; `map` lives in `tmx-core` and stays pure, and `TokioScheduler` keeps tokio confined to `tmx-adapters` (Task-01 purity gate).

## Obligations

- **O1 — A `map` over a collection runs its inner task per element and merges an array in item order regardless of completion order, with `concurrency` honoured up to the cap.**
  - *Claim:* `map` resolves `items` to an array, runs the inner task for each element (bound under `as` with `.index`) through `Scheduler.run_indexed`, and merges `state[name] = [ … ]` in item order regardless of completion order, honouring `concurrency` up to the cap.
  - *Evidence to collect:* Read `crates/tmx-core/src/fanout.rs` (`map`) and `crates/tmx-adapters/src/scheduler.rs` (`TokioScheduler`). Run a `map` over `docs/examples/map-fanout.yaml` (or `docs/examples/map-tasks.yaml`) under the `TokioScheduler` with `concurrency > 1` and expect item-ordered output.
  - *Checks:* Resolve the collection path to an index-ordered `Vec` via `Scheduler.run_indexed`, confirming output order follows item index, not completion order; confirm the in-flight count never exceeds the resolved `concurrency` (semaphore-bounded).
  - *Status:* ☐ unverified

- **O2 — An over-`FANOUT_WIDTH_MAX` resolved collection returns `fanout_too_wide` and an over-`CONCURRENCY_MAX` request is rejected, and the output-length assertion holds on both sides.**
  - *Claim:* An expression resolving to an array longer than `FANOUT_WIDTH_MAX` returns `fanout_too_wide`; a requested `concurrency` above `CONCURRENCY_MAX` (or below 1) is rejected; the output `Vec` length equals the input length, asserted on both the producing and consuming side.
  - *Evidence to collect:* Read the width and concurrency assertions in `crates/tmx-core/src/fanout.rs` and `crates/tmx-adapters/src/scheduler.rs`. Run the over-width test (expect `fanout_too_wide`), the over-concurrency test (expect rejection), and confirm the length assertions. Confirm `FANOUT_WIDTH_MAX` and `CONCURRENCY_MAX` are named units-last constants in `tmx-schema::limits`.
  - *Checks:* Trace the `items` resolution so an expression yielding an over-`FANOUT_WIDTH_MAX` array returns `fanout_too_wide` (a literal over-width is caught at preflight, an expression over-width here); confirm the output-length assertion holds on both the producing and consuming side.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm every new bound (`FANOUT_WIDTH_MAX`, `CONCURRENCY_MAX`) is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Run a `map` with `concurrency > 1` under the `TokioScheduler` and confirm item-ordered output, then run it under the `SerialScheduler` for identical results (Reviewable).**
  - *Claim:* A reviewer can run the same `map` under the `TokioScheduler` (`concurrency > 1`) and the `SerialScheduler` and observe identical, item-ordered output arrays.
  - *Evidence to collect:* Run the `map` (e.g. `docs/examples/map-fanout.yaml`) under the `TokioScheduler` with `concurrency > 1` and under the Task-06 `SerialScheduler`; observe byte-identical item-ordered output from both.
  - *Status:* ☐ unverified

## Regression check

- Task 18 extends the Task-11 runner with `map`: trace that a map-free flow still runs unchanged — run the Task-11 core integration test (the multi-task `assert`/`exec` flow with no `map`) and confirm it passes unchanged after the scheduler/fanout are added.

## Residue

`fanout.rs` lives in `tmx-core` and must stay pure; the `TokioScheduler` must keep tokio confined to `tmx-adapters` — confirm the Task-01 `cargo tree` purity gate still rejects a tokio edge into `tmx-core` (e.g. `cargo tree -p tmx-core -i tokio` finds nothing). The `SerialScheduler`-vs-`TokioScheduler` equivalence is the determinism check. The element error policy (`continueOnError` records the error in the slot; else abort the `map`) is a Step — confirm both arms.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
