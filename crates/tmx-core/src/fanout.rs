//! `map` — the bounded fan-out orchestration over the [`Scheduler`] port.
//!
//! [`run_map`] is the *only* non-sequential construct in the engine
//! ([05 §`map`](../../../.specs/05-fan-out-and-eval.md)): it runs a single inner task once per
//! element of a resolved collection and collects the per-element outputs into an **array**, always in
//! **item order** regardless of completion order. It is pure — it owns no I/O and spawns no work of
//! its own; all concurrency crosses the injected [`Scheduler`] port, and the actual per-element task
//! execution crosses the `run_item` callback the runner supplies. The production `TokioScheduler`
//! (in `tmx-adapters`) bounds real concurrent work; the test `SerialScheduler` runs strictly
//! serially — `run_map` is identical over either.
//!
//! The algorithm mirrors 05 §`map` step for step: resolve `items` to an array and bound its width by
//! [`FANOUT_WIDTH_MAX`] (`fanout_too_wide` on excess); bound the requested `concurrency` by
//! [`CONCURRENCY_MAX`] (`concurrency_too_high` on excess) and the caller's global cap; build each
//! element's binding (the element under `item`, with a synthetic `.index`); run the inner task per
//! element through [`Scheduler::run_indexed`]; collect the results in index order (asserting the
//! output length equals the input length on *both* the producing and consuming side, Tiger-Style
//! paired assertions); and apply the element error policy — `continueOnError` records a failing
//! element's error in its slot, otherwise the first failure aborts the whole `map`.

use std::future::Future;

use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::ChatMessage;
use tmx_schema::limits::{
    CONCURRENCY_MAX, EVAL_PASS_SCORE_DEFAULT_RATIO, FANOUT_WIDTH_MAX, FLOW_DEPTH_MAX,
};
use tmx_schema::task::{EvalWith, ExecWith, MapWith, RunWith, Scorer, TaskWith};

use crate::dispatch::{
    build_exec_spec, build_run_spec, interp_to_string, interp_value, split_args,
};
use crate::error::RunError;
use crate::matcher::MatcherEngine;
use crate::model::{EvalCase, EvalSummary, Scope, Scorecard};
use crate::ports::driven::{ChatModel, ChatRequest, ProcessRunner, Scheduler};

/// Run a `map` task's bounded fan-out and return the collected output **array** (item order).
///
/// `map` is the parsed `map` payload, `name` the task name (for typed-error attribution), `scope` the
/// parent run scope `items` is resolved against, `scheduler` the concurrency port, `concurrency_cap`
/// the run's global concurrency ceiling (the `--concurrency` flag; itself never above
/// [`CONCURRENCY_MAX`]), `depth` the current `flow`-recursion depth, and `run_item` the callback that
/// executes the inner task for one element — it receives the element's index, its `item` binding
/// (the element with a synthetic `.index` for object elements), and the depth the inner task runs at
/// (incremented when the inner task is a `flow`).
///
/// # Errors
///
/// - `fanout_too_wide` — the resolved `items` array is longer than [`FANOUT_WIDTH_MAX`] (an
///   expression over-width; a literal over-width is already rejected at preflight).
/// - `concurrency_too_high` — the requested `concurrency` exceeds [`CONCURRENCY_MAX`].
/// - `map_items_not_array` — `items` resolves to a value that is not an array.
/// - `flow_depth_exceeded` — a `flow` inner task would recurse past [`FLOW_DEPTH_MAX`].
/// - the aborting element's error — when an element fails and `continueOnError` is not set.
pub async fn run_map<S, F, Fut>(
    map: &MapWith,
    name: &str,
    scope: &Scope<'_>,
    scheduler: &S,
    concurrency_cap: u32,
    depth: u32,
    run_item: F,
) -> Result<Value, RunError>
where
    S: Scheduler,
    F: Fn(u32, Value, u32) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Value, RunError>> + Send,
{
    // 1. Resolve `items` to an array. An inline array interpolates element-wise; a lone
    //    `${{ expr }}` string resolves to the referenced value. Anything not an array is a typed
    //    error (the negative space of "iterate a collection").
    let resolved = interp_value(&map.items, scope)?;
    let items = resolved.as_array().ok_or_else(|| {
        RunError::run_failure(
            "map_items_not_array",
            format!("map task {name:?} `items` did not resolve to an array"),
        )
        .with_task(name)
    })?;
    let n = items.len();

    // Bound the fan-out width: "bounded iteration" is literally bounded (Tiger Style). An expression
    // that resolves to an over-limit array is caught here at runtime.
    if n as u64 > u64::from(FANOUT_WIDTH_MAX) {
        return Err(RunError::run_failure(
            "fanout_too_wide",
            format!(
                "map task {name:?} resolved {n} items, exceeding the {FANOUT_WIDTH_MAX} fan-out width limit"
            ),
        )
        .with_task(name));
    }
    // Backstop for the width bound now that the typed guard has passed.
    assert!(
        n as u64 <= u64::from(FANOUT_WIDTH_MAX),
        "fan-out width must be within FANOUT_WIDTH_MAX"
    );

    // 2. Resolve the concurrency budget. `concurrency` defaults to 1 (strictly in order); a request
    //    above the engine ceiling is a typed error, and the effective budget is further clamped by
    //    the run's global cap. It never drops below one unit (the Scheduler contract's lower bound).
    let requested = map.concurrency.unwrap_or(1);
    if requested > CONCURRENCY_MAX {
        return Err(RunError::validation(
            "concurrency_too_high",
            format!(
                "map task {name:?} requests concurrency {requested}, exceeding the {CONCURRENCY_MAX} ceiling"
            ),
        )
        .with_task(name));
    }
    // `CONCURRENCY_MAX >= 1` holds at compile time (a `limits` sanity assertion), so `clamp`'s
    // `min <= max` precondition is always met.
    let cap = concurrency_cap.clamp(1, CONCURRENCY_MAX);
    let effective = requested.max(1).min(cap);
    // The Scheduler contract's bounds, asserted before submit (paired with the adapter's own asserts).
    assert!(
        effective >= 1,
        "effective concurrency must be at least one unit"
    );
    assert!(
        effective <= CONCURRENCY_MAX,
        "effective concurrency must not exceed CONCURRENCY_MAX units"
    );

    // 3. A `flow` inner task consumes a recursion level (04 §Bounded flow recursion); a too-deep nest
    //    is a typed error *before* any element runs, mirroring the sequential dispatcher's guard.
    let inner_depth = if matches!(&map.task.with, TaskWith::Flow(_)) {
        if depth >= FLOW_DEPTH_MAX {
            return Err(RunError::resolution(
                "flow_depth_exceeded",
                format!(
                    "map task {name:?} flow inner task at depth {depth} would recurse past the {FLOW_DEPTH_MAX}-level bound"
                ),
            )
            .with_task(name));
        }
        depth + 1
    } else {
        depth
    };

    // 4. Run the inner task once per element through the Scheduler, collecting in INDEX order (not
    //    completion order). The Scheduler guarantees a length-`n` vector; we bind each element under
    //    `item` (with `.index`) and hand the callback the element and the inner depth.
    let run_item = &run_item;
    let results = scheduler
        .run_indexed(n as u32, effective, |index| {
            let element = bind_item(&items[index as usize], index);
            run_item(index, element, inner_depth)
        })
        .await;
    // Producing-side paired assertion: exactly one result per element, in index order.
    assert_eq!(
        results.len(),
        n,
        "the scheduler returns exactly one result per item, in index order"
    );

    // 5. Apply the element error policy while collecting the ordered output. `continueOnError` records
    //    a failing element's error in its slot (the same slot shape the sequential runner uses) and
    //    continues; otherwise the first failure aborts the whole `map`.
    let continue_on_error = map.continue_on_error.unwrap_or(false);
    let mut out: Vec<Value> = Vec::with_capacity(n);
    for result in results {
        match result {
            Ok(value) => out.push(value),
            Err(error) => {
                if continue_on_error {
                    out.push(serde_json::json!({
                        "error": serde_json::to_value(&error).unwrap_or(Value::Null),
                    }));
                } else {
                    return Err(error);
                }
            }
        }
    }
    // Consuming-side paired assertion: the output array length equals the input length.
    assert_eq!(
        out.len(),
        n,
        "the map output array holds exactly one slot per item"
    );

    Ok(Value::Array(out))
}

/// Bind one element as the inner task's scope *value*: the element itself, plus a synthetic `index`
/// for object elements so `${{ item.index }}` reads the element's position. A scalar or array element
/// is bound unchanged (it is used whole, e.g. `${{ item }}`); an object that already defines its own
/// `index` keeps it (the element's data wins over the synthetic key).
///
/// `.index` is *unconditional* across element types (04 §Interpolation namespaces), but a scalar or
/// array element has nowhere to hold a synthetic key — so for those the position is threaded to the
/// interpolator out-of-band via [`Scope::item_index`], which synthesises `${{ <alias>.index }}`. The
/// element's binding *root name* (the map's `as:` alias, default `item`) is likewise threaded via
/// [`Scope::item_alias`]; both are set by the runner alongside this value.
fn bind_item(element: &Value, index: u32) -> Value {
    match element {
        Value::Object(fields) => {
            let mut bound = fields.clone();
            bound.entry("index").or_insert_with(|| Value::from(index));
            Value::Object(bound)
        }
        other => other.clone(),
    }
}

// =============================================================================================
// `eval` — measurement over a dataset with scorers, a Scorecard summary, and threshold gating (05
// §`eval`, §Scorers). Like `map`, it is pure orchestration over the injected [`Scheduler`]: it owns
// no I/O and spawns no work, delegating the subject run to the `run_subject` callback and the
// side-effecting scorers to the injected [`ChatModel`] / [`ProcessRunner`] ports. The pure `matcher`
// scorer runs inline through the shared [`MatcherEngine`], never a parallel vocabulary.
// =============================================================================================

/// Run an `eval` task's measurement and return the [`Scorecard`] as a JSON [`Value`] for the merge.
///
/// `eval` is the parsed payload, `name` the task name (for typed-error attribution), `scope` the
/// parent run scope `dataset`/scorer operands resolve against, `scheduler` the concurrency port,
/// `chat`/`process` the ports the `llmRubric`/`exec` scorers cross, `concurrency_cap` the run's
/// global concurrency ceiling, `depth` the current `flow`-recursion depth, and `run_subject` the
/// callback that runs the `subject` once for one case (index, the `${{ case }}` binding, and the
/// depth the subject runs at — incremented when the subject is a `flow`).
///
/// Each case runs the `subject` once (when present), binds `${{ output }}` and `${{ case }}`, then
/// applies each scorer; the per-case score is the weighted mean of its scorers' scores (every score
/// asserted in `[0, 1]`). The `summary` aggregates `mean`, `weightedMean`, `passRate`, `min`, `p50`,
/// `p90`, and `count` over the per-case scores. A `threshold` (`metric >= min`) gates the run: a miss
/// is a `RunFailure` (`eval_threshold_missed`); without one the overall `passed` is `true`.
///
/// # Errors
///
/// - `fanout_too_wide` — the resolved `dataset` is longer than [`FANOUT_WIDTH_MAX`].
/// - `concurrency_too_high` — the requested `concurrency` exceeds [`CONCURRENCY_MAX`].
/// - `eval_dataset_not_array` — `dataset` resolves to a value that is not an array.
/// - `flow_depth_exceeded` — a `flow` subject would recurse past [`FLOW_DEPTH_MAX`].
/// - `scorer_bad_output` — an `exec`/`run` scorer emits output that is not a number in `[0, 1]`.
/// - `rubric_bad_output` — an `llmRubric` judge returns a non-conforming (non-`[0,1]`) response.
/// - `eval_threshold_missed` — the gated metric fell below the `threshold.min`.
/// - a scorer-configuration error (`unknown_scorer_type`, `scorer_missing_matcher`, …), or the
///   subject's own error.
#[allow(clippy::too_many_arguments)]
pub async fn run_eval<S, F, Fut>(
    eval: &EvalWith,
    name: &str,
    scope: &Scope<'_>,
    scheduler: &S,
    chat: &dyn ChatModel,
    process: &dyn ProcessRunner,
    concurrency_cap: u32,
    depth: u32,
    run_subject: F,
) -> Result<Value, RunError>
where
    S: Scheduler,
    F: Fn(u32, Value, u32) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Value, RunError>> + Send,
{
    // At least one scorer is required (the schema enforces it; assert the invariant defensively so a
    // caller that bypassed the loader cannot produce a meaningless empty-scorer weighted mean).
    if eval.scorers.is_empty() {
        return Err(RunError::validation(
            "eval_no_scorers",
            format!("eval task {name:?} declares no scorers"),
        )
        .with_task(name));
    }
    assert!(
        !eval.scorers.is_empty(),
        "an eval must carry at least one scorer"
    );

    // 1. Resolve the dataset: an explicit array of cases (each binds as `${{ case }}`), or a single
    //    synthetic case when `dataset` is absent (run once, no `${{ case }}` binding).
    let cases: Option<Vec<Value>> = match &eval.dataset {
        Some(dataset) => {
            let resolved = interp_value(dataset, scope)?;
            let array = resolved.as_array().ok_or_else(|| {
                RunError::run_failure(
                    "eval_dataset_not_array",
                    format!("eval task {name:?} `dataset` did not resolve to an array"),
                )
                .with_task(name)
            })?;
            Some(array.clone())
        }
        None => None,
    };
    let n = cases.as_ref().map_or(1usize, Vec::len);

    // Bound the fan-out width, exactly as `map` does: bounded iteration must be literally bounded.
    if n as u64 > u64::from(FANOUT_WIDTH_MAX) {
        return Err(RunError::run_failure(
            "fanout_too_wide",
            format!(
                "eval task {name:?} resolved {n} cases, exceeding the {FANOUT_WIDTH_MAX} fan-out width limit"
            ),
        )
        .with_task(name));
    }
    assert!(
        n as u64 <= u64::from(FANOUT_WIDTH_MAX),
        "fan-out width must be within FANOUT_WIDTH_MAX"
    );

    // 2. Resolve the concurrency budget (same clamp as `map`): default 1, capped by the engine
    //    ceiling and the run's global cap, never below one in-flight unit.
    let requested = eval.concurrency.unwrap_or(1);
    if requested > CONCURRENCY_MAX {
        return Err(RunError::validation(
            "concurrency_too_high",
            format!(
                "eval task {name:?} requests concurrency {requested}, exceeding the {CONCURRENCY_MAX} ceiling"
            ),
        )
        .with_task(name));
    }
    let cap = concurrency_cap.clamp(1, CONCURRENCY_MAX);
    let effective = requested.max(1).min(cap);
    assert!(
        effective >= 1,
        "effective concurrency must be at least one unit"
    );
    assert!(
        effective <= CONCURRENCY_MAX,
        "effective concurrency must not exceed CONCURRENCY_MAX units"
    );

    // 3. A `flow` subject consumes a recursion level; guard before any case runs (as `map` does).
    let subject_present = eval.subject.is_some();
    let inner_depth = match &eval.subject {
        Some(task) if matches!(task.with, TaskWith::Flow(_)) => {
            if depth >= FLOW_DEPTH_MAX {
                return Err(RunError::resolution(
                    "flow_depth_exceeded",
                    format!(
                        "eval task {name:?} flow subject at depth {depth} would recurse past the {FLOW_DEPTH_MAX}-level bound"
                    ),
                )
                .with_task(name));
            }
            depth + 1
        }
        _ => depth,
    };

    // The per-case `passed` flag colours cases against `passScore` (default 0.5, overridden by a
    // threshold's `passScore`); `threshold.metric` — separately — gates the run (05 §Decisions).
    let pass_score = eval
        .threshold
        .as_ref()
        .and_then(|t| t.pass_score)
        .unwrap_or(EVAL_PASS_SCORE_DEFAULT_RATIO);

    // 4. Run each case through the Scheduler, collecting scored cases in INDEX order. Any case error
    //    (a subject failure, a bad scorer output) aborts the whole eval — there is no per-case
    //    continue policy: a scorecard with a silently-dropped case would misreport the metrics.
    let scorers = &eval.scorers;
    let cases_ref = cases.as_ref();
    let run_subject = &run_subject;
    let results = scheduler
        .run_indexed(n as u32, effective, |index| {
            let case_value = cases_ref.map(|c| c[index as usize].clone());
            score_case(
                index,
                case_value,
                scorers,
                scope,
                chat,
                process,
                pass_score,
                subject_present,
                inner_depth,
                run_subject,
                name,
            )
        })
        .await;
    assert_eq!(
        results.len(),
        n,
        "the scheduler returns exactly one result per case, in index order"
    );

    let mut scored: Vec<EvalCase> = Vec::with_capacity(n);
    for result in results {
        scored.push(result?);
    }
    assert_eq!(
        scored.len(),
        n,
        "the scorecard holds exactly one case per dataset entry"
    );

    // 5. Aggregate the summary and apply the threshold gate.
    let summary = aggregate_summary(&scored, pass_score);
    let passed = match &eval.threshold {
        None => true,
        Some(threshold) => {
            let metric = threshold.metric.as_deref().unwrap_or("weightedMean");
            let value = metric_value(&summary, metric).ok_or_else(|| {
                RunError::validation(
                    "unknown_eval_metric",
                    format!("eval task {name:?} threshold references unknown metric {metric:?}"),
                )
                .with_task(name)
            })?;
            if value < threshold.min {
                return Err(RunError::run_failure(
                    "eval_threshold_missed",
                    format!(
                        "eval task {name:?} metric {metric} = {value} is below the required minimum {}",
                        threshold.min
                    ),
                )
                .with_task(name));
            }
            true
        }
    };

    let scorecard = Scorecard {
        cases: scored,
        summary,
        passed,
    };
    // Paired assertion: the scorecard reports one case per dataset entry and a defined pass verdict.
    assert_eq!(
        scorecard.cases.len(),
        n,
        "the emitted scorecard carries one case per dataset entry"
    );
    assert_eq!(
        scorecard.summary.count as usize, n,
        "the summary count equals the number of scored cases"
    );

    serde_json::to_value(&scorecard).map_err(|e| {
        RunError::run_failure(
            "eval_scorecard_unserializable",
            format!("eval task {name:?} scorecard did not serialise: {e}"),
        )
        .with_task(name)
    })
}

/// Score one dataset case: run the `subject` (when present), then apply every scorer and fold their
/// scores into the case's weighted-mean score. Returns the [`EvalCase`] for the scorecard.
#[allow(clippy::too_many_arguments)]
async fn score_case<F, Fut>(
    index: u32,
    case_value: Option<Value>,
    scorers: &[Scorer],
    parent_scope: &Scope<'_>,
    chat: &dyn ChatModel,
    process: &dyn ProcessRunner,
    pass_score: f64,
    subject_present: bool,
    inner_depth: u32,
    run_subject: &F,
    name: &str,
) -> Result<EvalCase, RunError>
where
    F: Fn(u32, Value, u32) -> Fut,
    Fut: Future<Output = Result<Value, RunError>> + Send,
{
    // Run the subject once for this case (when present); its output binds as `${{ output }}`.
    let output_value: Option<Value> = if subject_present {
        let case_arg = case_value.clone().unwrap_or(Value::Null);
        Some(run_subject(index, case_arg, inner_depth).await?)
    } else {
        None
    };

    // The case scope: the parent bindings plus this case's `${{ case }}` and `${{ output }}`.
    let case_scope = Scope {
        case: case_value.as_ref(),
        output: output_value.as_ref(),
        ..*parent_scope
    };

    let mut scores: IndexMap<String, f64> = IndexMap::with_capacity(scorers.len());
    let mut weighted_sum = 0.0f64;
    let mut weight_sum = 0.0f64;
    for scorer in scorers {
        let weight = scorer.weight.unwrap_or(1.0);
        if !(weight.is_finite() && weight > 0.0) {
            return Err(RunError::validation(
                "scorer_bad_weight",
                format!(
                    "scorer {:?} in eval task {name:?} has non-positive weight {weight}",
                    scorer.name
                ),
            )
            .with_task(name));
        }
        let score = score_one(
            scorer,
            &case_scope,
            output_value.as_ref(),
            chat,
            process,
            name,
        )
        .await?;
        // The `[0, 1]` invariant is checked before the score is folded into the weighted mean — the
        // scorer paths already reject out-of-range values, so this is the paired backstop.
        assert!(
            (0.0..=1.0).contains(&score) && score.is_finite(),
            "every scorer score must be within [0, 1]"
        );
        scores.insert(scorer.name.clone(), score);
        weighted_sum += score * weight;
        weight_sum += weight;
    }
    // Every scorer contributes a positive weight, so the divisor is positive (no divide-by-zero).
    assert!(
        weight_sum > 0.0,
        "the summed scorer weight must be positive"
    );
    let case_score = weighted_sum / weight_sum;
    assert!(
        (0.0..=1.0).contains(&case_score) && case_score.is_finite(),
        "the per-case weighted-mean score must be within [0, 1]"
    );

    Ok(EvalCase {
        case: case_value,
        output: output_value,
        scores,
        score: case_score,
        passed: case_score >= pass_score,
    })
}

/// Evaluate a single scorer against the case scope, returning its score in `[0, 1]`.
///
/// The `matcher` scorer is pure ([`MatcherEngine`], `1.0`/`0.0`); `llmRubric` crosses the
/// [`ChatModel`] port; `exec`/`run` crosses the [`ProcessRunner`] port. A judge or command that does
/// not yield a number in `[0, 1]` is a typed failure, never a silent zero.
async fn score_one(
    scorer: &Scorer,
    case_scope: &Scope<'_>,
    output_value: Option<&Value>,
    chat: &dyn ChatModel,
    process: &dyn ProcessRunner,
    name: &str,
) -> Result<f64, RunError> {
    let kind = scorer.scorer_type.as_deref().unwrap_or("matcher");
    match kind {
        "matcher" => {
            let actual = resolve_actual(scorer, case_scope, output_value, name)?;
            let matcher = scorer.matcher.ok_or_else(|| {
                RunError::validation(
                    "scorer_missing_matcher",
                    format!(
                        "matcher scorer {:?} in eval task {name:?} has no matcher",
                        scorer.name
                    ),
                )
                .with_task(name)
            })?;
            let expected = scorer
                .expected
                .as_ref()
                .map(|e| interp_value(e, case_scope))
                .transpose()?;
            let args = split_args(matcher, expected.as_ref());
            let not = scorer.not.unwrap_or(false);
            let held = MatcherEngine::evaluate(&actual, matcher, args.as_deref(), not);
            Ok(if held { 1.0 } else { 0.0 })
        }
        "llmRubric" => {
            let actual = resolve_actual(scorer, case_scope, output_value, name)?;
            let model = scorer.model.clone().ok_or_else(|| {
                RunError::validation(
                    "rubric_missing_model",
                    format!(
                        "llmRubric scorer {:?} in eval task {name:?} has no model",
                        scorer.name
                    ),
                )
                .with_task(name)
            })?;
            let rubric = match &scorer.rubric {
                Some(r) => value_to_text(&interp_value(&Value::String(r.clone()), case_scope)?),
                None => String::new(),
            };
            // Route the judge call to the scorer's configured endpoint/key when set (interpolated —
            // an `apiKey` typically references a context secret); absent, the adapter's composed
            // default is used.
            let api_url = scorer
                .api_url
                .as_deref()
                .map(|u| interp_to_string(u, case_scope))
                .transpose()?;
            let api_key = scorer
                .api_key
                .as_deref()
                .map(|k| interp_to_string(k, case_scope))
                .transpose()?;
            let request = ChatRequest {
                model,
                messages: vec![
                    ChatMessage {
                        role: "system".to_string(),
                        content: Value::String(format!(
                            "You are grading an output against a rubric. {rubric}\nReply with only a single number between 0 and 1."
                        )),
                        name: None,
                    },
                    ChatMessage {
                        role: "user".to_string(),
                        content: Value::String(value_to_text(&actual)),
                        name: None,
                    },
                ],
                temperature: None,
                max_tokens: None,
                api_url,
                api_key,
            };
            let response = chat.complete(request).await?;
            unit_score(parse_score(&response.content)).ok_or_else(|| {
                RunError::run_failure(
                    "rubric_bad_output",
                    format!(
                        "llmRubric scorer {:?} in eval task {name:?} judge returned {:?}, not a number in [0, 1]",
                        scorer.name, response.content
                    ),
                )
                .with_task(name)
            })
        }
        "exec" | "run" => {
            let with = scorer.with.as_ref().ok_or_else(|| {
                RunError::validation(
                    "scorer_missing_with",
                    format!(
                        "{kind} scorer {:?} in eval task {name:?} has no `with`",
                        scorer.name
                    ),
                )
                .with_task(name)
            })?;
            let empty_env = IndexMap::new();
            let spec = if kind == "run" {
                let rw: RunWith = serde_json::from_value(with.clone()).map_err(|e| {
                    RunError::validation(
                        "scorer_bad_with",
                        format!("run scorer {:?} `with` is invalid: {e}", scorer.name),
                    )
                    .with_task(name)
                })?;
                build_run_spec(&rw, name, case_scope, &empty_env)?
            } else {
                let ew: ExecWith = serde_json::from_value(with.clone()).map_err(|e| {
                    RunError::validation(
                        "scorer_bad_with",
                        format!("exec scorer {:?} `with` is invalid: {e}", scorer.name),
                    )
                    .with_task(name)
                })?;
                build_exec_spec(&ew, name, case_scope, &empty_env)?
            };
            let out = process.run(spec).await?;
            let bad = || {
                RunError::run_failure(
                    "scorer_bad_output",
                    format!(
                        "{kind} scorer {:?} in eval task {name:?} did not emit a number in [0, 1]",
                        scorer.name
                    ),
                )
                .with_task(name)
            };
            if out.exit_code != Some(0) {
                return Err(bad());
            }
            let text = String::from_utf8_lossy(&out.stdout);
            unit_score(parse_score(&text)).ok_or_else(bad)
        }
        other => Err(RunError::validation(
            "unknown_scorer_type",
            format!(
                "scorer {:?} in eval task {name:?} has unknown type {other:?}",
                scorer.name
            ),
        )
        .with_task(name)),
    }
}

/// Resolve a scorer's `actual` value: its explicit (interpolated) `actual`, or the subject's
/// `${{ output }}` by default. An eval with no subject and a scorer with no `actual` is a typed error
/// (the value to score is undetermined) rather than a silent `null`.
fn resolve_actual(
    scorer: &Scorer,
    case_scope: &Scope<'_>,
    output_value: Option<&Value>,
    name: &str,
) -> Result<Value, RunError> {
    match &scorer.actual {
        Some(actual) => interp_value(actual, case_scope),
        None => output_value.cloned().ok_or_else(|| {
            RunError::resolution(
                "scorer_missing_actual",
                format!(
                    "scorer {:?} in eval task {name:?} has no `actual` and the eval has no subject output to score",
                    scorer.name
                ),
            )
            .with_task(name)
        }),
    }
}

/// Aggregate the per-case scores into the [`EvalSummary`] — every metric an `evalThreshold` can gate.
///
/// `mean` and `weightedMean` are both the arithmetic mean of the per-case scores: scorer weights are
/// already folded into each case score, and v0 cases carry no case-level weight, so the two coincide.
/// Both are emitted because `evalThreshold.metric` may gate on either (`weightedMean` is the default).
/// `p50`/`p90` use the **nearest-rank** percentile method over the ascending-sorted scores.
fn aggregate_summary(cases: &[EvalCase], pass_score: f64) -> EvalSummary {
    let count = cases.len();
    if count == 0 {
        return EvalSummary {
            mean: 0.0,
            weighted_mean: 0.0,
            pass_rate: 0.0,
            min: None,
            p50: None,
            p90: None,
            count: 0,
        };
    }
    let scores: Vec<f64> = cases.iter().map(|c| c.score).collect();
    let sum: f64 = scores.iter().sum();
    let mean = sum / count as f64;
    let passed = cases.iter().filter(|c| c.score >= pass_score).count();
    let pass_rate = passed as f64 / count as f64;
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = sorted.first().copied();
    EvalSummary {
        mean,
        weighted_mean: mean,
        pass_rate,
        min,
        p50: percentile_nearest_rank(&sorted, 0.5),
        p90: percentile_nearest_rank(&sorted, 0.9),
        count: count as u32,
    }
}

/// The nearest-rank percentile of an ascending-sorted slice: `rank = ceil(fraction × n)`, clamped to
/// `[1, n]`, returning `sorted[rank - 1]`. `None` for an empty slice (no cases scored).
fn percentile_nearest_rank(sorted_ascending: &[f64], fraction: f64) -> Option<f64> {
    let n = sorted_ascending.len();
    if n == 0 {
        return None;
    }
    debug_assert!(
        (0.0..=1.0).contains(&fraction),
        "a percentile fraction must be within [0, 1]"
    );
    let rank = (fraction * n as f64).ceil() as usize;
    let rank = rank.clamp(1, n);
    sorted_ascending.get(rank - 1).copied()
}

/// Look up an `evalThreshold` metric by its schema name, `None` for an unknown name. `min`/`p50`/`p90`
/// are `0.0` when absent (an empty dataset), so a gate on them fails rather than panics.
fn metric_value(summary: &EvalSummary, metric: &str) -> Option<f64> {
    match metric {
        "mean" => Some(summary.mean),
        "weightedMean" => Some(summary.weighted_mean),
        "passRate" => Some(summary.pass_rate),
        "min" => Some(summary.min.unwrap_or(0.0)),
        "p50" => Some(summary.p50.unwrap_or(0.0)),
        "p90" => Some(summary.p90.unwrap_or(0.0)),
        _ => None,
    }
}

/// Parse a scorer's raw text output into a number: a bare number (`0.9`), or a `{ "score": 0.9 }`
/// object. Returns the raw value (range-checked separately by [`unit_score`]); `None` when the text
/// is not a number or a `score` object.
fn parse_score(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        match value {
            Value::Number(n) => return n.as_f64(),
            Value::Object(map) => {
                return map.get("score").and_then(Value::as_f64);
            }
            _ => {}
        }
    }
    trimmed.parse::<f64>().ok()
}

/// Keep a parsed score only when it is a finite number within `[0, 1]`; `None` otherwise.
fn unit_score(raw: Option<f64>) -> Option<f64> {
    raw.filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
}

/// Render a JSON value to text for a rubric prompt / an `actual` a judge reads: a string yields its
/// contents, any other value its compact JSON form.
fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use serde_json::json;

    use crate::ports::driven::Scheduler;

    /// A minimal in-crate serial [`Scheduler`] for the unit tests: it runs `make(0..count)` one at a
    /// time, in index order. Defined locally rather than reusing `tmx-testkit`'s `SerialScheduler`
    /// because a `#[cfg(test)]` unit module compiled *into* `tmx-core` sees the dev-dependency's view
    /// of `tmx-core` as a distinct crate instance (the classic cyclic-dev-dep two-versions problem);
    /// the cross-adapter equivalence with the real schedulers is covered by `tmx-adapters`' tests.
    struct TestSerialScheduler;

    impl Scheduler for TestSerialScheduler {
        async fn run_indexed<T, F, Fut>(
            &self,
            count: u32,
            concurrency: u32,
            make: F,
        ) -> Vec<Result<T, RunError>>
        where
            T: Send,
            F: Fn(u32) -> Fut + Send + Sync,
            Fut: Future<Output = Result<T, RunError>> + Send,
        {
            assert!(concurrency >= 1, "concurrency must be at least one unit");
            assert!(
                concurrency <= CONCURRENCY_MAX,
                "concurrency must not exceed CONCURRENCY_MAX units"
            );
            let mut out = Vec::with_capacity(count as usize);
            for index in 0..count {
                out.push(make(index).await);
            }
            assert_eq!(out.len(), count as usize, "one result per index, in order");
            out
        }
    }

    /// Drive an immediately-ready future with a no-op waker — the workspace's purity-preserving
    /// pattern (no async runtime linked into the pure core's tests). `run_map` over the serial
    /// scheduler with immediately-ready `run_item` callbacks completes on the first poll.
    fn block_on_ready<Fut: Future>(fut: Fut) -> Fut::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    /// An empty run scope — `map`'s `items` here are inline literals, so no namespace is read.
    fn empty_scope() -> (Value, Value) {
        (
            Value::Object(serde_json::Map::new()),
            Value::Object(serde_json::Map::new()),
        )
    }

    fn scope_over<'a>(empty: &'a Value) -> Scope<'a> {
        Scope {
            inputs: empty,
            env: empty,
            secrets: empty,
            tasks: empty,
            item: None,
            item_alias: None,
            item_index: None,
            case: None,
            output: None,
            matrix: empty,
        }
    }

    /// Build a `MapWith` from JSON — the inner `task` is a valid (but here unexecuted) task object.
    fn map_with(value: Value) -> MapWith {
        serde_json::from_value(value).expect("valid MapWith fixture")
    }

    #[test]
    fn collects_outputs_in_item_order() {
        // Each element is echoed back through `run_item`; the collected array must follow item order.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a", "b", "c"],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out =
            block_on_ready(run_map(
                &map,
                "fan",
                &scope,
                &TestSerialScheduler,
                CONCURRENCY_MAX,
                0,
                |index, element, _depth| async move {
                    Ok(json!({ "index": index, "element": element }))
                },
            ))
            .expect("the map runs every element");
        let array = out.as_array().expect("the map output is an array");
        assert_eq!(array.len(), 3, "one output slot per item");
        assert_eq!(
            array[0],
            json!({ "index": 0, "element": "a" }),
            "the first slot is the first item's output"
        );
        assert_eq!(
            array[2],
            json!({ "index": 2, "element": "c" }),
            "the last slot is the last item's output, in item order"
        );
    }

    #[test]
    fn binds_the_element_and_a_synthetic_index_for_object_elements() {
        // An object element is bound under `item` with a synthetic `.index`; a scalar is bound whole.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": [{ "sku": "x1" }, { "sku": "x2" }],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            // `run_item` receives the already-bound `item` value; echo it so the binding is asserted.
            |_index, item, _depth| async move { Ok(item) },
        ))
        .expect("the map runs");
        let array = out.as_array().expect("array output");
        assert_eq!(
            array[0],
            json!({ "sku": "x1", "index": 0 }),
            "the first object element carries its own field and a synthetic index 0"
        );
        assert_eq!(
            array[1],
            json!({ "sku": "x2", "index": 1 }),
            "the second element's synthetic index is its position"
        );
    }

    #[test]
    fn an_over_width_expression_is_fanout_too_wide() {
        // An `items` array longer than FANOUT_WIDTH_MAX is a typed `fanout_too_wide` RunFailure.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let over = vec![Value::Null; (FANOUT_WIDTH_MAX as usize) + 1];
        let map = map_with(json!({
            "items": Value::Array(over),
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect_err("an over-width fan-out is rejected");
        assert_eq!(
            err.code, "fanout_too_wide",
            "the width error carries its code"
        );
        assert_eq!(
            err.task.as_deref(),
            Some("fan"),
            "the error names the offending map task"
        );
    }

    #[test]
    fn an_over_concurrency_request_is_rejected() {
        // A requested concurrency above CONCURRENCY_MAX is a typed `concurrency_too_high` error,
        // rejected before any element runs.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a"],
            "concurrency": CONCURRENCY_MAX + 1,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect_err("an over-concurrency request is rejected");
        assert_eq!(
            err.code, "concurrency_too_high",
            "the concurrency error carries its code"
        );
        assert_eq!(err.task.as_deref(), Some("fan"), "the error names the task");
    }

    #[test]
    fn continue_on_error_records_the_error_in_the_slot() {
        // With `continueOnError`, a failing element records its error in its own slot and the map
        // completes with a full-length output array.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["ok0", "bad1", "ok2"],
            "continueOnError": true,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |index, _element, _depth| async move {
                if index == 1 {
                    Err(RunError::run_failure("element_boom", "element one failed"))
                } else {
                    Ok(json!(index))
                }
            },
        ))
        .expect("continueOnError keeps the map running past a failing element");
        let array = out.as_array().expect("array output");
        assert_eq!(array.len(), 3, "every element still holds a slot");
        assert_eq!(array[0], json!(0), "the first element's output is recorded");
        assert_eq!(
            array[1]["error"]["code"], "element_boom",
            "the failing element's error is recorded in its own slot"
        );
        assert_eq!(array[2], json!(2), "iteration continued after the failure");
    }

    #[test]
    fn a_failing_element_aborts_the_map_without_continue_on_error() {
        // Without `continueOnError`, the first failing element aborts the whole map with its error.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["ok0", "bad1", "ok2"],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |index, _element, _depth| async move {
                if index == 1 {
                    Err(RunError::run_failure("element_boom", "element one failed"))
                } else {
                    Ok(json!(index))
                }
            },
        ))
        .expect_err("a failing element aborts the map");
        assert_eq!(
            err.code, "element_boom",
            "the abort surfaces the failing element's error"
        );
    }

    #[test]
    fn items_that_do_not_resolve_to_an_array_is_a_typed_error() {
        // Negative space: `items` resolving to a non-array value is `map_items_not_array`.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": 42,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect_err("a non-array items is rejected");
        assert_eq!(
            err.code, "map_items_not_array",
            "the error names the non-array items"
        );
    }

    #[test]
    fn an_empty_collection_yields_an_empty_array() {
        // A zero-width fan-out is valid: it runs no elements and merges an empty array.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": [],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect("an empty map runs");
        assert_eq!(out, json!([]), "an empty collection yields an empty array");
    }

    #[test]
    fn a_flow_inner_task_increments_the_inner_depth() {
        // A `flow` inner task runs one recursion level deeper; the callback observes depth + 1.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a"],
            "task": { "type": "flow", "with": { "use": "./sub.yaml" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            3,
            |_index, _element, depth| async move { Ok(json!(depth)) },
        ))
        .expect("the flow inner task runs");
        assert_eq!(
            out,
            json!([4]),
            "a flow inner task at map depth 3 runs its body at depth 4"
        );
    }

    #[test]
    fn a_flow_inner_task_at_the_depth_limit_is_rejected() {
        // Negative space: a `flow` inner task at FLOW_DEPTH_MAX would recurse past the bound.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a"],
            "task": { "type": "flow", "with": { "use": "./sub.yaml" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            FLOW_DEPTH_MAX,
            |_index, _element, depth| async move { Ok(json!(depth)) },
        ))
        .expect_err("a too-deep flow inner task is rejected");
        assert_eq!(
            err.code, "flow_depth_exceeded",
            "the depth guard fires before any element runs"
        );
    }

    #[test]
    fn a_leaf_inner_task_keeps_the_map_depth() {
        // A non-flow inner task does not consume a recursion level: the callback observes the map's
        // own depth unchanged (paired with the flow-increments test above).
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a", "b"],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            2,
            |_index, _element, depth| async move { Ok(json!(depth)) },
        ))
        .expect("the leaf inner task runs");
        assert_eq!(
            out,
            json!([2, 2]),
            "a leaf inner task keeps the map's depth"
        );
    }

    // -----------------------------------------------------------------------------------------
    // `eval` pure-aggregation unit tests. The port-crossing `run_eval` paths (matcher/llmRubric/
    // exec scorers, threshold gating over the real fakes) are exercised in the integration test
    // `tests/eval.rs`, which can inject `tmx-testkit`'s fakes as the `tmx-core` port traits (an
    // in-crate `#[cfg(test)]` module sees the dev-dependency's `tmx-core` as a distinct instance).
    // -----------------------------------------------------------------------------------------

    /// Build a scored [`EvalCase`] carrying just `score` — the aggregation helpers read only `score`.
    fn case_scored(score: f64, pass_score: f64) -> EvalCase {
        EvalCase {
            case: None,
            output: None,
            scores: IndexMap::new(),
            score,
            passed: score >= pass_score,
        }
    }

    #[test]
    fn aggregate_summary_carries_every_gateable_metric() {
        // Four scores → mean, weightedMean (== mean in v0), passRate, min, p50, p90, count. The
        // nearest-rank percentiles: p50 of 4 → rank ceil(2.0)=2 → sorted[1]; p90 → rank ceil(3.6)=4.
        let cases = [
            case_scored(0.2, 0.5),
            case_scored(0.8, 0.5),
            case_scored(0.4, 0.5),
            case_scored(1.0, 0.5),
        ];
        let summary = aggregate_summary(&cases, 0.5);
        assert_eq!(summary.count, 4, "one entry per scored case");
        assert!(
            (summary.mean - 0.6).abs() < 1e-9,
            "mean is the arithmetic mean of the case scores"
        );
        assert_eq!(
            summary.weighted_mean, summary.mean,
            "weightedMean coincides with mean when cases carry no case-level weight"
        );
        assert!(
            (summary.pass_rate - 0.5).abs() < 1e-9,
            "two of four cases are at or above passScore 0.5"
        );
        assert_eq!(summary.min, Some(0.2), "min is the smallest case score");
        assert_eq!(
            summary.p50,
            Some(0.4),
            "nearest-rank p50 of [0.2,0.4,0.8,1.0] is sorted[1] = 0.4"
        );
        assert_eq!(
            summary.p90,
            Some(1.0),
            "nearest-rank p90 of four scores is sorted[3] = 1.0"
        );
    }

    #[test]
    fn aggregate_summary_of_no_cases_reports_empty_metrics() {
        // Negative space: an empty dataset yields zeroed scalar metrics and absent percentiles,
        // rather than a divide-by-zero NaN.
        let summary = aggregate_summary(&[], 0.5);
        assert_eq!(summary.count, 0, "no cases were scored");
        assert_eq!(summary.mean, 0.0, "the empty mean is defined as zero");
        assert_eq!(summary.min, None, "no min without a case");
        assert_eq!(summary.p50, None, "no p50 without a case");
        assert_eq!(summary.p90, None, "no p90 without a case");
    }

    #[test]
    fn a_single_case_reduces_every_metric_to_its_score() {
        // Residue: a single-case (single-scorer) eval reduces every aggregate to that one score.
        let summary = aggregate_summary(&[case_scored(0.7, 0.5)], 0.5);
        assert_eq!(summary.mean, 0.7, "mean of one score is that score");
        assert_eq!(summary.min, Some(0.7), "min of one score is that score");
        assert_eq!(summary.p50, Some(0.7), "p50 of one score is that score");
        assert_eq!(summary.p90, Some(0.7), "p90 of one score is that score");
    }

    #[test]
    fn percentile_nearest_rank_is_defined_and_bounded() {
        let sorted = [0.1, 0.3, 0.5, 0.7, 0.9];
        assert_eq!(
            percentile_nearest_rank(&sorted, 0.5),
            Some(0.5),
            "p50 of five scores is rank ceil(2.5)=3 → sorted[2]"
        );
        assert_eq!(
            percentile_nearest_rank(&sorted, 0.9),
            Some(0.9),
            "p90 of five scores is rank ceil(4.5)=5 → sorted[4]"
        );
        assert_eq!(
            percentile_nearest_rank(&[], 0.5),
            None,
            "an empty slice has no percentile"
        );
    }

    #[test]
    fn parse_and_range_check_accept_numbers_and_reject_non_numbers() {
        // A bare number, a `{ "score": x }` object, and whitespace all parse; range-checking then
        // keeps only `[0,1]`.
        assert_eq!(unit_score(parse_score("0.9")), Some(0.9), "a bare number");
        assert_eq!(
            unit_score(parse_score("  {\"score\": 0.25}  ")),
            Some(0.25),
            "a score object, trimmed"
        );
        assert_eq!(unit_score(parse_score("0")), Some(0.0), "the lower bound");
        assert_eq!(unit_score(parse_score("1")), Some(1.0), "the upper bound");

        // Negative space: non-numeric text, an out-of-range number, and an empty string are rejected
        // (so a scorer returns a typed error, never a silent zero).
        assert_eq!(
            unit_score(parse_score("great job")),
            None,
            "prose is not a score"
        );
        assert_eq!(unit_score(parse_score("1.5")), None, "above the unit range");
        assert_eq!(
            unit_score(parse_score("-0.1")),
            None,
            "below the unit range"
        );
        assert_eq!(
            unit_score(parse_score("")),
            None,
            "empty output is not a score"
        );
        assert_eq!(
            unit_score(parse_score("{\"other\": 1}")),
            None,
            "an object without a `score` key is not a score"
        );
    }

    #[test]
    fn metric_value_maps_every_gateable_name_and_rejects_unknown() {
        let summary = EvalSummary {
            mean: 0.6,
            weighted_mean: 0.6,
            pass_rate: 0.5,
            min: Some(0.2),
            p50: Some(0.4),
            p90: Some(1.0),
            count: 4,
        };
        assert_eq!(
            metric_value(&summary, "weightedMean"),
            Some(0.6),
            "default metric"
        );
        assert_eq!(metric_value(&summary, "min"), Some(0.2), "min is gateable");
        assert_eq!(metric_value(&summary, "p90"), Some(1.0), "p90 is gateable");
        // Negative space: an out-of-vocabulary metric has no value (a typed error at the gate).
        assert_eq!(
            metric_value(&summary, "median"),
            None,
            "an unknown metric name is rejected"
        );
    }
}
