# Task 19 — `eval` measurement and scorers

**Plan:** [plan.md](../plan.md) · **Certificate:** [19-eval_and_scorers-certificate.md](19-eval_and_scorers-certificate.md)

**Implements:** [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §`eval` — measurement, §Scorers, §Flow / sequence
**Depends on:** 08, 18
**Produces:** the `eval` task — measurement over a dataset with `matcher`/`llmRubric`/`exec` scorers, a `Scorecard` summary carrying every gateable metric, and threshold gating
**Pointers:** `crates/tmx-core/src/fanout.rs` (the `eval` path), `crates/tmx-core/src/model.rs` (`Scorecard`/`EvalCase`)

## Steps

- [x] Resolve `dataset` (or a single synthetic case), assert `len <= FANOUT_WIDTH_MAX`, and run each case through the `Scheduler`: run the `subject` once (if present), bind `${{ output }}` and `${{ case }}`, then apply each scorer.
- [x] Implement the three scorer kinds — `matcher` (pure `MatcherEngine` → 1.0/0.0), `llmRubric` (via the `ChatModel` port), `exec`/`run` (via the `ProcessRunner` port, parsing a number in `[0,1]`, else `scorer_bad_output`) — computing the per-case weighted mean and asserting each score is in `[0,1]`.
- [x] Aggregate the `summary` (`mean`, `weightedMean`, `passRate`, `min`, `p50`, `p90`, `count`) with a defined percentile method, and set per-case `passed` against `passScore` (default 0.5).
- [x] Apply the `threshold` (`metric >= min`): a miss is a `RunFailure` (`eval_threshold_missed`); without a threshold the overall `passed` is `true`; merge `state[name] = { cases, summary, passed }`.

## Definition of done

- [x] An `eval` over a dataset emits a `Scorecard` whose summary carries every gateable metric, and a missed `threshold` fails the run while a threshold-less `eval` only reports.
- [x] Each scorer score is asserted within `[0,1]`, an `exec` scorer emitting a non-`[0,1]` number returns `scorer_bad_output`, and a non-conforming `llmRubric` response is a `RunFailure` rather than a silent zero (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run an `eval` with a mixed scorer set and a threshold, and confirm the scorecard metrics and the pass/fail gate over the fakes.
