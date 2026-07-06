# Done Certificate — Task 19: `eval` measurement and scorers

**Task:** [19-eval_and_scorers.md](19-eval_and_scorers.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

> This certificate is a verification protocol for Task 19. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 19) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The `eval` task measures over a dataset with `matcher`/`llmRubric`/`exec` scorers, emits a `Scorecard` summary carrying every gateable metric, and gates on a `threshold`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** `eval` extends the Task 18 fan-out over the same `Scheduler` port; adding the eval path must not change the existing `map` run path (item-ordered output, `FANOUT_WIDTH_MAX` width cap).

## Obligations

- **O1 — An `eval` over a dataset emits a `Scorecard` whose summary carries every gateable metric; a missed `threshold` fails the run while a threshold-less `eval` only reports.**
  - *Claim:* an `eval` over a dataset produces a `Scorecard` whose `summary` carries `mean`, `weightedMean`, `passRate`, `min`, `p50`, `p90`, and `count`; a `threshold` miss is a `RunFailure` (`eval_threshold_missed`); with no `threshold` the overall `passed` is `true` and `state[name] = { cases, summary, passed }` merges.
  - *Evidence to collect:* read the `eval` path in `crates/tmx-core/src/fanout.rs` and the `Scorecard`/`EvalCase` types in `crates/tmx-core/src/model.rs`; run the named eval test over the `tmx-testkit` fakes (`FakeChatModel`, `RecordingProcessRunner`, `SerialScheduler`) and assert the summary carries all seven metrics; run one flow whose `threshold` metric sits above the achieved value (expect `RunFailure` `eval_threshold_missed`) and one with no `threshold` (expect `passed = true` and the merged `state[name]`).
  - *Checks:* trace that `summary` aggregates over the collected per-case scores with the task's defined percentile method for `p50`/`p90`; trace that `threshold` (`metric >= min`) is the only gate and the threshold-less path sets `passed = true`.
  - *Status:* ☑ SATISFIED — `run_eval` (`crates/tmx-core/src/fanout.rs`) emits a `Scorecard` whose `EvalSummary` carries all seven metrics; `metric_value` covers exactly the schema's `evalThreshold.metric` enum (`mean`/`weightedMean`/`passRate`/`min`/`p50`/`p90`, verified against `docs/tmx.schema.json`); percentiles use the documented nearest-rank method (unit-tested). Integration tests `a_mixed_scorer_eval_emits_a_full_scorecard_and_gates_on_the_threshold`, `the_threshold_gate_flips_when_the_minimum_crosses_the_achieved_metric` (miss → `RunFailure` `eval_threshold_missed`, task-attributed), and `a_threshold_less_eval_only_reports_and_passes` all pass (nextest, 2026-07-06).

- **O2 — Each scorer score is asserted within `[0,1]`; an `exec` scorer emitting a non-`[0,1]` number returns `scorer_bad_output`; a non-conforming `llmRubric` response is a `RunFailure` rather than a silent zero (negative space).**
  - *Claim:* every scorer score is checked within `[0,1]` before the per-case weighted mean; an `exec` scorer whose parsed number falls outside `[0,1]` returns `scorer_bad_output`; a non-conforming `llmRubric` judge response is a `RunFailure`, not `0.0`.
  - *Evidence to collect:* run the negative-space tests over the fakes — a `RecordingProcessRunner` scripted to emit a number outside `[0,1]` (expect `scorer_bad_output`) and a `FakeChatModel` scripted with a malformed/non-numeric judge response (expect `RunFailure`, not `0.0`); read the `[0,1]` assertion sited before the weighted mean in `crates/tmx-core/src/fanout.rs`.
  - *Checks:* assert each scorer score is clamped/checked in `[0,1]` before the weighted mean; resolve the `matcher` scorer to the pure `MatcherEngine` (Task 08), the `llmRubric` scorer to the `ChatModel` port, and the `exec`/`run` scorer to the `ProcessRunner` port.
  - *Status:* ☑ SATISFIED — `unit_score` rejects (never clamps) non-finite/out-of-range values before `score_case`'s paired `[0,1]` assert and the weighted mean; `an_exec_scorer_emitting_a_non_unit_number_is_scorer_bad_output` (exec emits 4.2 → `scorer_bad_output`) and `a_non_conforming_llm_rubric_response_is_a_run_failure_not_a_zero` (prose judge → `rubric_bad_output`, non-Validation category) both pass. Mutation probe: replacing `unit_score`'s reject with a clamp made both the unit test and the integration test FAIL (guards are load-bearing); probe reverted, suite green. Scorer routing confirmed: `matcher` → `MatcherEngine::evaluate` via the shared `split_args`, `llmRubric` → `ChatModel::complete`, `exec`/`run` → `ProcessRunner::run` via the shared `build_exec_spec`/`build_run_spec`.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm any new bound (the dataset-length ceiling reusing `FANOUT_WIDTH_MAX`, the default `passScore` of 0.5) is a named constant in `tmx-schema::limits`, not a literal. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☑ SATISFIED — validator-run 2026-07-06: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 234/234 passed; `scripts/purity.sh` intact. `EVAL_PASS_SCORE_DEFAULT_RATIO` (units-last, compile-time range-asserted) added to `tmx-schema::limits`; dataset width reuses `FANOUT_WIDTH_MAX`; concurrency reuses `CONCURRENCY_MAX`; flow depth reuses `FLOW_DEPTH_MAX`.

- **O4 — Run an `eval` with a mixed scorer set and a threshold, and confirm the scorecard metrics and the pass/fail gate over the fakes (Reviewable).**
  - *Claim:* a reviewer can run an `eval` with a mixed scorer set (`matcher` + `llmRubric` + `exec`) and a `threshold` over the `tmx-testkit` fakes and observe the scorecard metrics and the pass/fail gate flipping as the threshold crosses the achieved metric.
  - *Evidence to collect:* run the reviewable eval flow/test over the fake bundle; observe the `Scorecard` summary (all seven metrics) and that `passed` flips true↔false as the `threshold` is set below/above the achieved metric.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -E 'binary(eval)'`: 5/5 passed. The mixed-scorer test drives `matcher` + `llmRubric` + `exec` over `SerialScheduler`/`FakeChatModel`/`RecordingProcessRunner`, asserts all seven summary metrics present, per-scorer scores (1.0 / 1.0 / 0.8), weightedMean ≈ 0.8, and the gate; the flip test shows `min: 0.5` → passed and `min: 1.5` → `eval_threshold_missed`.

## Regression check

- Extending the Task 18 fan-out with the eval path; trace that a `map` flow (Task 18 test) still runs unchanged over the `Scheduler` — item-ordered output and the `FANOUT_WIDTH_MAX` width cap intact : ☑ PRESERVED — the diff leaves `run_map` untouched (imports only); `cargo nextest run -E 'test(map) or test(fanout) or test(eval)'` 32/32 passed, including the Task 18 ordering/width tests and the tokio-vs-serial scheduler equivalence. The `dispatch.rs` visibility promotions (`split_args`, `build_exec_spec`, `build_run_spec` → `pub(crate)`) change no behaviour; all downstream `assert`/`exec`/`run` dispatch tests pass.

## Residue

- The percentile method for `p50`/`p90` is "a defined method" per the task step, not a specific one — the validator should confirm the method is documented and consistently applied, not merely that values appear.
- The per-case `subject` runs once when present, then binds `${{ output }}`/`${{ case }}`; confirm a dataset case with no `subject` still scores.
- Weighted-mean weighting across scorers: confirm the default weight and that a single-scorer `eval` reduces to that scorer's score.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED and the map regression PRESERVED. `run_eval` delivers dataset-bounded measurement with the three scorer kinds over the right ports, a seven-metric `Scorecard` matching the schema's gateable enum exactly, nearest-rank percentiles, `passScore` case colouring, and `metric >= min` threshold gating (miss → `eval_threshold_missed`). Negative space (`scorer_bad_output`, `rubric_bad_output`, out-of-range/prose parses, unknown metric, empty dataset) is typed and test-covered; a validator mutation probe (clamp instead of reject in `unit_score`) tripped both the unit and integration guards and was reverted. Gates run 2026-07-06: fmt clean, clippy `-D warnings` clean, nextest 234/234, purity intact. Residue discharged: percentile method documented on `aggregate_summary`/`percentile_nearest_rank`; a subject-less scorer requires explicit `actual` (`scorer_missing_actual`, no silent null); default weight 1.0 with positive-weight validation, and a single-case eval reduces every aggregate to its score. Note (non-blocking): the `Map | Eval` dispatch arm still returns `task_type_unsupported` — runner wiring is a later task, consistent with Task 18's precedent and this task's Pointers.
