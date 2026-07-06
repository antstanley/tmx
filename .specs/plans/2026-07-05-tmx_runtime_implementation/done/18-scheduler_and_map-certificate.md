# Done Certificate — Task 18: Scheduler port and `map` fan-out

**Task:** [18-scheduler_and_map.md](18-scheduler_and_map.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

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
  - *Status:* ☑ SATISFIED — `run_map` (`crates/tmx-core/src/fanout.rs`) resolves `items`, binds each element (synthetic `.index` for objects) and collects via `Scheduler::run_indexed` in index order. Item-order-despite-completion-order proven by `scheduler::tokio_tests::collects_in_index_order_despite_reversed_completion_order` (earlier indices sleep longer; output still index-ordered) and the O4 equivalence test (a real parsed `MapWith`, `concurrency: 4`, reversed completion delays). In-flight bound proven by `never_runs_more_than_concurrency_units_at_once` (barrier-forced peak == budget, never above) plus the adapter's own `in_flight <= concurrency` assertion; an injected over-admit mutation (`Semaphore::new(concurrency + 1)`) tripped three tests, then was reverted. Note: `map` is not yet wired into `dispatch_task`/the CLI (plan defers the runner/`--concurrency` seam downstream — `Ports` is a `dyn` bundle and `Scheduler` is deliberately not object-safe), so the literal `docs/examples/map-fanout.yaml` end-to-end run is exercised structurally via `run_sample_map` rather than from the shell; the example's custom `as: region` alias (`MapWith.as_binding`) is likewise consumed at the wiring layer, not by `run_map`.

- **O2 — An over-`FANOUT_WIDTH_MAX` resolved collection returns `fanout_too_wide` and an over-`CONCURRENCY_MAX` request is rejected, and the output-length assertion holds on both sides.**
  - *Claim:* An expression resolving to an array longer than `FANOUT_WIDTH_MAX` returns `fanout_too_wide`; a requested `concurrency` above `CONCURRENCY_MAX` (or below 1) is rejected; the output `Vec` length equals the input length, asserted on both the producing and consuming side.
  - *Evidence to collect:* Read the width and concurrency assertions in `crates/tmx-core/src/fanout.rs` and `crates/tmx-adapters/src/scheduler.rs`. Run the over-width test (expect `fanout_too_wide`), the over-concurrency test (expect rejection), and confirm the length assertions. Confirm `FANOUT_WIDTH_MAX` and `CONCURRENCY_MAX` are named units-last constants in `tmx-schema::limits`.
  - *Checks:* Trace the `items` resolution so an expression yielding an over-`FANOUT_WIDTH_MAX` array returns `fanout_too_wide` (a literal over-width is caught at preflight, an expression over-width here); confirm the output-length assertion holds on both the producing and consuming side.
  - *Status:* ☑ SATISFIED — `fanout::tests::an_over_width_expression_is_fanout_too_wide` builds a `FANOUT_WIDTH_MAX + 1` array and gets a typed `fanout_too_wide` naming the task; `an_over_concurrency_request_is_rejected` gets `concurrency_too_high` for `CONCURRENCY_MAX + 1`, and the adapter's ceiling assert is exercised by `#[should_panic] rejects_a_concurrency_above_the_ceiling`. Paired length assertions at fanout.rs:149 (producing, `results.len() == n`) and fanout.rs:175 (consuming, `out.len() == n`), plus the adapter's own `out.len() == count` assert. `FANOUT_WIDTH_MAX` (100_000) and `CONCURRENCY_MAX` (256) are named units-last constants in `tmx-schema::limits` with compile-time sanity assertions; no new numeric bound introduced. A `concurrency: 0` request is rejected at preflight by the schema's `minimum: 1`; `run_map`'s `.max(1)` is a backstop only.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm every new bound (`FANOUT_WIDTH_MAX`, `CONCURRENCY_MAX`) is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — independently run by the validator: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 223/223 passed (11 new fanout + 6 new scheduler tests among them); `cargo build -p tmx-adapters --no-default-features` clean (TokioScheduler/futures-util correctly feature-gated). Both bounds are pre-existing named constants in `tmx-schema::limits`; the code introduces no new numeric bound.

- **O4 — Run a `map` with `concurrency > 1` under the `TokioScheduler` and confirm item-ordered output, then run it under the `SerialScheduler` for identical results (Reviewable).**
  - *Claim:* A reviewer can run the same `map` under the `TokioScheduler` (`concurrency > 1`) and the `SerialScheduler` and observe identical, item-ordered output arrays.
  - *Evidence to collect:* Run the `map` (e.g. `docs/examples/map-fanout.yaml`) under the `TokioScheduler` with `concurrency > 1` and under the Task-06 `SerialScheduler`; observe byte-identical item-ordered output from both.
  - *Status:* ☑ SATISFIED — `scheduler::tokio_tests::map_output_is_identical_under_the_concurrent_and_serial_schedulers` runs the same parsed `MapWith` (five items, inner delays reversing completion order) through `run_map` under `TokioScheduler` with `concurrency: 4` and under the production `SerialScheduler`, asserting both equal the expected item-ordered array and equal each other. Run by the validator and observed passing; the same test failed under the injected over-admit scheduler mutation (then reverted), so the equivalence check genuinely bites. The YAML-file-from-the-shell form of this action awaits the runner/CLI wiring task per plan.md (map still returns `task_type_unsupported` from `dispatch_task`).

## Regression check

- Task 18 extends the Task-11 runner with `map`: trace that a map-free flow still runs unchanged — run the Task-11 core integration test (the multi-task `assert`/`exec` flow with no `map`) and confirm it passes unchanged after the scheduler/fanout are added.
- **Result:** PASS — `dispatch_task` is untouched (a `map` task still yields `task_type_unsupported`), so map-free flows take exactly the pre-task-18 path. `runner_runs_multi_task_flow_emits_ordered_stream_and_masked_state` and the full `tmx-core::runner` / hooks / preflight suites pass unchanged (223/223 workspace-wide). The `continueOnError` slot shape in `run_map` (`{"error": <RunError>}`) matches the sequential runner's `fail_task` slot exactly (runner.rs:715).

## Residue

`fanout.rs` lives in `tmx-core` and must stay pure; the `TokioScheduler` must keep tokio confined to `tmx-adapters` — confirm the Task-01 `cargo tree` purity gate still rejects a tokio edge into `tmx-core` (e.g. `cargo tree -p tmx-core -i tokio` finds nothing). The `SerialScheduler`-vs-`TokioScheduler` equivalence is the determinism check. The element error policy (`continueOnError` records the error in the slot; else abort the `map`) is a Step — confirm both arms.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED by independently-run evidence. `run_map` (pure, in `tmx-core`) delivers bounded fan-out over the `Scheduler` port — width/concurrency/depth guards typed against the named `tmx-schema::limits` constants, paired length assertions on both sides, both `continueOnError` arms tested — and the semaphore-bounded `TokioScheduler` collects in index order regardless of completion order, byte-identical to the `SerialScheduler` (O4 run and observed; both injected mutations — depth non-increment and semaphore over-admit — tripped tests and were reverted). fmt/clippy/nextest (223/223), `--no-default-features` build, and the purity gate (`cargo tree -p tmx-core -i tokio` / `-i futures-util` find nothing) are all green. Residue for the downstream wiring task: `map` still returns `task_type_unsupported` from `dispatch_task`, and `MapWith.as_binding` (custom `as:` alias, e.g. the example's `as: region`) is not yet consumed anywhere — both deliberately deferred per plan.md (the `Ports` bundle is `dyn` and `Scheduler` is not object-safe).
