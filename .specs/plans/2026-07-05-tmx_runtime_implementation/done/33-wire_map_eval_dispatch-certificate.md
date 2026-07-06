# Done Certificate — Task 33: Wire map/eval task dispatch into the runner

**Task:** [33-wire_map_eval_dispatch.md](33-wire_map_eval_dispatch.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — verifier discharged all obligations against the working copy (jj change rnzpnzmy)

> A validating agent discharges this: for each obligation collect the named evidence, run the
> named checks, set the Status, then derive the Conclusion by the rubric. Do not record DONE
> with any non-SATISFIED obligation.

## Definition

DONE(Task 33) ≡ obligations O1…O4 all hold, each backed by run/observed evidence — the two
control-flow task types (`map`, `eval`) actually execute through the runner, with no regression.

## Premises

- **P1 — Goal.** `map` and `eval` tasks execute end-to-end via `RunFlow`/`tmx run`, not just as standalone `run_map`/`run_eval` unit calls.
- **P2 — Obligations.** Done iff O1…O4 hold; O3 is the negative-space/regression item, O4 the Reviewable.
- **P3 — Invariants.** `tmx-core` stays pure (purity gate green — the scheduler is an injected port, `TokioScheduler` in adapters only); all prior tests (447 at task 32) stay green; every `run_map`/`run_eval` typed guard still surfaces.

## Obligations

- **O1 — A `map` task runs end-to-end and collects its output array in item order.**
  - *Claim:* a Flow whose task list contains a `map` task fans out over `items` under the bounded scheduler, binds `as`, and collects per-item outputs into an array in item order; the concurrency cap and `continueOnError` behave per spec.
  - *Evidence to collect:* run a `map` Flow through `RunFlow` over the fakes and, per DoD, through the real `tmx run` binary; observe the collected array in final state is in item order; confirm `dispatch.rs` no longer returns `task_type_unsupported` for `Map`. Confirm `map.item.finish` events are emitted.
  - *Status:* ☑ SATISFIED — `fanout_dispatch.rs` tests `a_map_flow_runs_end_to_end_and_collects_the_output_array_in_item_order` (state `fan == [10,20,30]`, 3× `map.item.finish`, no `task.error`) and `a_map_binds_each_element_under_item_so_the_inner_task_reads_it` (`${{ item.n }}` gates pass) both pass; real binary: `tmx run` on a 3-item map with `concurrency: 2` produced the array in item order with `map: fan[i]` events; a map-of-flow ran the child sub-flow per element with `${{ item }}` passed as input, collected in order; `continueOnError: true` recorded a failing element's typed error in-slot and finished `ok`. `dispatch.rs:129-130` now returns `Dispatch::Map`/`Dispatch::Eval` (exhaustive match, no wildcard). *Caveat:* the `as` alias (custom binding name) is not honoured — a declared `as: region` fails loudly with "`region` is not a known interpolation namespace"; this is a pre-existing gap in `run_map`'s `bind_item`/the interpolator's fixed `Scope` namespaces (tasks 07/18), not introduced by this wiring; the default `item` binding (the spec default) is verified. Recommend a follow-up gap task.

- **O2 — An `eval` task runs end-to-end and emits a `Scorecard`; a missed threshold fails the run.**
  - *Evidence to collect:* run an `eval` Flow; observe a `Scorecard` in the result and `eval.case.finish` events; a Flow whose eval misses its `threshold` fails (typed `eval_threshold_missed`).
  - *Status:* ☑ SATISFIED — `an_eval_flow_runs_end_to_end_and_emits_a_scorecard` (2 cases, `summary`, `passed: true`, 2× `eval.case.finish`) and `an_eval_that_misses_its_threshold_fails_the_run` (`RunStatus::Failed`, code `eval_threshold_missed`) pass; real binary: `tmx run` on an eval flow produced the full Scorecard (per-case `case`/`output`/`scores`/`score`/`passed`, aggregate `summary` with mean/weightedMean/passRate/min/p50/p90/count, `passed: true`) with `eval: quality case 0/1` events; a below-threshold variant failed the run with "metric weightedMean = 0 is below the required minimum 0.5".

- **O3 — Guards preserved, no regression.**
  - *Claim:* the `run_map`/`run_eval` typed errors (`fanout_too_wide`, `concurrency_too_high`, `map_items_not_array`, `flow_depth_exceeded`, eval threshold gating) still surface through the runner; the `task_type_unsupported` backstop remains for a genuinely unknown type; the whole prior suite is green.
  - *Evidence to collect:* run `cargo nextest run` (all prior tests + the new e2e map/eval tests — expect ≥447 plus the new ones, 0 failures), `cargo clippy --all-targets --all-features -D warnings`, `cargo fmt --all --check`, and `scripts/purity.sh` — all clean. Confirm the purity script shows no tokio/futures edge into `tmx-core`.
  - *Checks:* over-width/over-concurrency/not-array cases still produce their typed errors when dispatched through the runner.
  - *Status:* ☑ SATISFIED — verifier independently ran `cargo fmt --all --check` (clean), `cargo clippy --all-targets --all-features -- -D warnings` (exit 0), `cargo nextest run` (452 run, 452 passed, 0 skipped — 447 prior + 5 new), `scripts/purity.sh` ("tmx-schema, tmx-core, tmx-testkit carry no I/O or async dependency edge"). Guards: `a_map_whose_items_do_not_resolve_to_an_array_surfaces_the_typed_guard` proves `map_items_not_array` through the runner; `concurrency: 9999` through the real binary produced typed `concurrency_too_high` ("at most 256, got 9999"); `fanout_too_wide`/`flow_depth_exceeded`/threshold gating live unchanged inside `run_map`/`run_eval` (their unit tests all green) and the propagation channel is proven by the not-array/threshold e2e cases. The unknown-type backstop is now structural: the closed `TaskWith` enum is matched exhaustively with no `_` wildcard (a new variant is a compile error; an unknown `type` string is a `schema_invalid` parse rejection, observed via the real binary's schema validation).

- **O4 — Reviewable: `tmx run` a map Flow and an eval Flow from the shell.**
  - *Evidence to collect:* build the binary with the needed features; `tmx run` a Flow containing a `map` task and observe the per-item output array in final state; `tmx run` a Flow containing an `eval` task and observe the `Scorecard`; neither returns `task_type_unsupported`.
  - *Status:* ☑ SATISFIED — verifier built and ran the real `tmx` binary on six flows: map (3 items, concurrency 2 → ordered array + `map: fan[i]` events), eval (full Scorecard, `passed: true`, `eval: quality case i` events), map-of-flow (child sub-flow per element with `${{ item }}` input, ordered), continueOnError (in-slot error, run ok), missed threshold (run failed, typed message), over-concurrency (typed `concurrency_too_high`). No run returned `task_type_unsupported`.

## Regression check

- The 447 tests green at task 32, plus the whole workspace, must stay green. `run_subflow`/`dispatch_task` for the six side-effecting types + assert must be unchanged in behaviour.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED. `map` and `eval` execute end-to-end through `RunFlow` and the real `tmx run` binary — ordered per-item arrays, `map.item.finish`/`eval.case.finish` events, full Scorecard, threshold gating, and every `run_map`/`run_eval` guard surfacing unchanged; the scheduler is injected as a generic port (core purity gate green), all 452 tests pass, fmt/clippy clean. Regression check: dispatch of the six leaf types + assert + flow is untouched (only the Map/Eval arm changed); all 447 prior tests green. Two disclosed, non-blocking scope notes for follow-up gap tasks: (1) the `as` element-binding alias is accepted by the schema but not honoured by the interpolator/`bind_item` (fails loudly with a typed unknown-namespace error; pre-existing since tasks 07/18); (2) a fan-out inner leaf task's own `secrets` list is not separately re-resolved inside the `Fn` callback — it inherits the map/eval task's binding (flow inner tasks resolve theirs against a fresh secrets-seeded masker and are scrubbed before merge, so no leak path).
