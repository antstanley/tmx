//! `run_eval` end-to-end over the deterministic `tmx-testkit` fakes.
//!
//! Task 19's reviewable path (05 §`eval`, §Scorers): an `eval` over a dataset with a mixed scorer
//! set (`matcher` + `llmRubric` + `exec`) and a `threshold`, driven over the `SerialScheduler`,
//! `FakeChatModel`, and `RecordingProcessRunner`. This test crate depends on the real `tmx-core` and
//! `tmx-testkit`, so both refer to the *same* `tmx-core` instance and the fakes satisfy its port
//! traits (unlike an in-crate `#[cfg(test)]` module, which sees a distinct dev-dependency instance).

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::model::Scope;
use tmx_core::ports::driven::ProcessOutput;
use tmx_core::{Milliseconds, run_eval};
use tmx_schema::limits::CONCURRENCY_MAX;
use tmx_schema::task::EvalWith;
use tmx_testkit::{FakeChatModel, RecordingProcessRunner, SerialScheduler};

/// Drive an immediately-ready future with a no-op waker — the workspace's purity-preserving pattern
/// (the fakes and the serial scheduler complete on the first poll, so no async runtime is linked).
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake-backed eval future must be immediately ready"),
    }
}

/// An empty parent scope — the eval's `dataset`/scorer operands here are inline literals.
fn empty_scope(empty: &Value) -> Scope<'_> {
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

// A test-only fixture builder (not itself a `#[test]` fn), so `allow-expect-in-tests` does not reach
// it: the `expect` panics only on a malformed fixture, which is the test author's error.
#[allow(clippy::expect_used)]
fn eval_with(value: Value) -> EvalWith {
    serde_json::from_value(value).expect("valid EvalWith fixture")
}

/// Script a process runner to emit `stdout` (exit 0) for each entry, in order.
fn runner_emitting(lines: &[&str]) -> RecordingProcessRunner {
    let runner = RecordingProcessRunner::new();
    for line in lines {
        runner.push_result(Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: line.as_bytes().to_vec(),
            stderr: Vec::new(),
            ms: Milliseconds(0),
        }));
    }
    runner
}

#[test]
fn a_mixed_scorer_eval_emits_a_full_scorecard_and_gates_on_the_threshold() {
    // Two cases; each scored by three scorers: a `matcher` (equality), an `llmRubric` (judge), and
    // an `exec` (a command emitting a number). The subject echoes each case's `expected` string.
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);

    // Judge returns 1.0 then 0.6; exec returns 0.8 then 0.4. Two calls per case in scorer order.
    let chat = FakeChatModel::new()
        .with_completion("1.0")
        .with_completion("0.6");
    let process = runner_emitting(&["{\"score\": 0.8}", "0.4"]);

    let eval = eval_with(json!({
        "dataset": [{ "expected": "hello" }, { "expected": "world" }],
        "subject": { "type": "exec", "with": { "command": "echo" } },
        "scorers": [
            { "name": "exact", "type": "matcher", "matcher": "toEqual", "expected": "${{ case.expected }}" },
            { "name": "judge", "type": "llmRubric", "model": "gpt-x", "rubric": "grade it" },
            { "name": "script", "type": "exec", "with": { "command": "score.sh" } },
        ],
        "threshold": { "metric": "weightedMean", "min": 0.5 },
    }));

    // The subject "runs" by echoing the case's expected string as the output to score.
    let out = block_on_ready(run_eval(
        &eval,
        "quality",
        &scope,
        &SerialScheduler::new(),
        &chat,
        &process,
        CONCURRENCY_MAX,
        0,
        |_index, case, _depth| async move {
            let expected = case.get("expected").cloned().unwrap_or(Value::Null);
            Ok(expected)
        },
    ))
    .expect("the mixed-scorer eval runs and meets its threshold");

    // Case 0: matcher 1.0 (echoed output == expected), judge 1.0, exec 0.8 → mean 0.9333…
    // Case 1: matcher 1.0, judge 0.6, exec 0.4 → mean 0.6667. weightedMean over cases = 0.8 ≥ 0.5.
    let scores = &out["summary"];
    for metric in [
        "mean",
        "weightedMean",
        "passRate",
        "min",
        "p50",
        "p90",
        "count",
    ] {
        assert!(
            scores.get(metric).is_some(),
            "the summary carries the gateable metric {metric}"
        );
    }
    assert_eq!(
        out["passed"],
        json!(true),
        "the achieved weightedMean clears the threshold"
    );
    assert_eq!(
        out["cases"].as_array().map(Vec::len),
        Some(2),
        "one scorecard entry per case"
    );
    assert_eq!(
        out["cases"][0]["scores"]["exact"],
        json!(1.0),
        "case 0's matcher scorer passed (output equals the expected string)"
    );
    assert_eq!(
        out["cases"][0]["scores"]["judge"],
        json!(1.0),
        "case 0's rubric score is the judge's parsed number"
    );
    assert_eq!(
        out["cases"][0]["scores"]["script"],
        json!(0.8),
        "case 0's exec score parses the score object the command emitted"
    );
    let mean = out["summary"]["weightedMean"]
        .as_f64()
        .expect("weightedMean is a number");
    assert!(
        (mean - 0.8).abs() < 1e-6,
        "weightedMean is the mean of the two case scores"
    );
}

#[test]
fn the_threshold_gate_flips_when_the_minimum_crosses_the_achieved_metric() {
    // The same achieved metric (0.8) fails once the required minimum is raised above it: the gate is
    // `metric >= min`, and a miss is a typed `eval_threshold_missed` RunFailure (05 §`eval`).
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);

    let build = |min: f64| {
        eval_with(json!({
            "subject": { "type": "exec", "with": { "command": "echo" } },
            "scorers": [{ "name": "s", "type": "matcher", "matcher": "toBeTruthy" }],
            "threshold": { "metric": "mean", "min": min },
        }))
    };

    // A single synthetic case (no dataset); the subject output is truthy → score 1.0, mean 1.0.
    let run = |min: f64| {
        let chat = FakeChatModel::new();
        let process = RecordingProcessRunner::new();
        let eval = build(min);
        block_on_ready(run_eval(
            &eval,
            "gate",
            &scope,
            &SerialScheduler::new(),
            &chat,
            &process,
            CONCURRENCY_MAX,
            0,
            |_i, _case, _d| async move { Ok(json!("non-empty")) },
        ))
    };

    let below = run(0.5).expect("a min below the achieved mean passes");
    assert_eq!(
        below["passed"],
        json!(true),
        "0.5 ≤ 1.0, so the gate passes"
    );

    let above = run(1.5).expect_err("a min above the achievable maximum fails the run");
    assert_eq!(
        above.code, "eval_threshold_missed",
        "a missed threshold is a typed RunFailure"
    );
    assert_eq!(
        above.task.as_deref(),
        Some("gate"),
        "the failure names the eval task"
    );
}

#[test]
fn a_threshold_less_eval_only_reports_and_passes() {
    // O1: without a threshold the overall `passed` is `true` — eval measures, it does not gate.
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    let chat = FakeChatModel::new();
    let process = RecordingProcessRunner::new();
    let eval = eval_with(json!({
        "subject": { "type": "exec", "with": { "command": "echo" } },
        // A matcher that fails (output is not the number 5) → score 0.0, but no threshold gates.
        "scorers": [{ "name": "s", "type": "matcher", "matcher": "toEqual", "expected": 5 }],
    }));
    let out = block_on_ready(run_eval(
        &eval,
        "report",
        &scope,
        &SerialScheduler::new(),
        &chat,
        &process,
        CONCURRENCY_MAX,
        0,
        |_i, _case, _d| async move { Ok(json!("text")) },
    ))
    .expect("a threshold-less eval never fails on score");
    assert_eq!(
        out["passed"],
        json!(true),
        "no threshold → overall passed is true"
    );
    assert_eq!(
        out["summary"]["mean"],
        json!(0.0),
        "the failing matcher still reports a zero score in the summary"
    );
    assert_eq!(
        out["cases"][0]["passed"],
        json!(false),
        "the case is coloured failing against passScore"
    );
}

#[test]
fn an_exec_scorer_emitting_a_non_unit_number_is_scorer_bad_output() {
    // O2 negative space: an exec scorer whose parsed number is outside [0,1] is `scorer_bad_output`,
    // not a clamped or silently-zeroed score.
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    let chat = FakeChatModel::new();
    let process = runner_emitting(&["4.2"]);
    let eval = eval_with(json!({
        "subject": { "type": "exec", "with": { "command": "echo" } },
        "scorers": [{ "name": "script", "type": "exec", "with": { "command": "score.sh" } }],
    }));
    let err = block_on_ready(run_eval(
        &eval,
        "bad",
        &scope,
        &SerialScheduler::new(),
        &chat,
        &process,
        CONCURRENCY_MAX,
        0,
        |_i, _case, _d| async move { Ok(json!("x")) },
    ))
    .expect_err("an out-of-range exec score is rejected");
    assert_eq!(
        err.code, "scorer_bad_output",
        "the code names the bad scorer output"
    );
    assert_eq!(
        err.task.as_deref(),
        Some("bad"),
        "the error names the eval task"
    );
}

#[test]
fn a_non_conforming_llm_rubric_response_is_a_run_failure_not_a_zero() {
    // O2 negative space: a judge that returns prose (no parseable number) is a RunFailure, never a
    // silent 0.0 that would quietly drag the mean down.
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    let chat = FakeChatModel::new().with_completion("The output looks great overall!");
    let process = RecordingProcessRunner::new();
    let eval = eval_with(json!({
        "subject": { "type": "exec", "with": { "command": "echo" } },
        "scorers": [{ "name": "judge", "type": "llmRubric", "model": "gpt-x", "rubric": "grade" }],
    }));
    let err = block_on_ready(run_eval(
        &eval,
        "judged",
        &scope,
        &SerialScheduler::new(),
        &chat,
        &process,
        CONCURRENCY_MAX,
        0,
        |_i, _case, _d| async move { Ok(json!("text")) },
    ))
    .expect_err("a non-numeric judge response is rejected");
    assert_eq!(
        err.code, "rubric_bad_output",
        "a non-conforming judge response is a typed failure"
    );
    assert_ne!(
        err.category,
        tmx_core::ErrorCategory::Validation,
        "it is a run failure over the judge output, not a static validation error"
    );
}
