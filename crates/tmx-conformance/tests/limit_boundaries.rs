//! Limit-boundary conformance tier (Task 32, O2).
//!
//! Every tunable and structural limit named in the task carries a **one-below / at / one-above**
//! trio here, each computed *from* its named `tmx-schema::limits` constant (never a hard-coded
//! literal) so the trio tracks the constant automatically: `STATE_SIZE_MAX_BYTES`,
//! `JSON_DEPTH_MAX`, `FLOW_DEPTH_MAX`, `TASKS_PER_FLOW_MAX`, `FANOUT_WIDTH_MAX`, `EXPR_LEN_MAX_BYTES`,
//! and `EXPR_DEPTH_MAX`. The below/at cases pass; the above case fails **closed** with the documented
//! typed error naming the limit — never a panic, never a silent truncation.
//!
//! The two size/width limits whose named ceiling is impractical to materialise byte-for-byte
//! (`STATE_SIZE_MAX_BYTES` at 512 MiB) are exercised against a small *configured* cap — the same
//! seam `--max-state-size` narrows — and separately pinned to their named constant, exactly as the
//! `tmx-core` unit tests do.

use indexmap::IndexMap;
use serde_json::{Value, json};

use tmx_conformance::{Bundle, block_on_ready, empty_scope};
use tmx_core::{
    ErrorCategory, Masker, PipelineRunner, RunConfig, RunId, RunStatus, StateBuilder, evaluate,
    matrix_combinations, resolve_flow,
};
use tmx_schema::limits::{
    EXPR_DEPTH_MAX, EXPR_LEN_MAX_BYTES, FANOUT_WIDTH_MAX, FLOW_DEPTH_MAX, JSON_DEPTH_MAX,
    STATE_SIZE_MAX_BYTES, TASKS_PER_FLOW_MAX,
};
use tmx_testkit::SerialScheduler;

const A_RUN_ID: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

/// A well-formed run id for driving `PipelineRunner` directly.
fn run_id() -> RunId {
    RunId::new(A_RUN_ID).unwrap_or_else(|_| panic!("the fixed conformance run id is valid"))
}

// ---------------------------------------------------------------------------------------------
// STATE_SIZE_MAX_BYTES — byte-precise, against a small configured cap (the `--max-state-size` seam).
// ---------------------------------------------------------------------------------------------

#[test]
fn state_size_boundary_below_at_above_a_configured_cap() {
    // The empty state is `{}` (2 bytes); the first element `"k":"<payload>"` adds the key token, a
    // colon, and the value's two quotes plus its payload. Derive the payload length that lands the
    // state exactly on a chosen cap, then probe one below / at / one above.
    const KEY: &str = "k";
    const CAP: u64 = 256;
    let key_token: u64 = serde_json::to_string(KEY)
        .map(|s| s.len() as u64)
        .unwrap_or(0);
    let overhead: u64 = 2 + key_token + 1 + 2; // {} + "k" + : + the two value quotes
    let at_cap_payload = (CAP - overhead) as usize;

    let mut below = StateBuilder::with_cap(CAP);
    below
        .merge(
            KEY,
            Value::String("x".repeat(at_cap_payload - 1)),
            "producer",
        )
        .unwrap_or_else(|_| panic!("one byte below the cap succeeds"));
    assert_eq!(
        below.size_bytes(),
        CAP - 1,
        "the below-cap merge lands one byte under the cap"
    );

    let mut at = StateBuilder::with_cap(CAP);
    at.merge(KEY, Value::String("x".repeat(at_cap_payload)), "producer")
        .unwrap_or_else(|_| panic!("exactly at the cap succeeds"));
    assert_eq!(
        at.size_bytes(),
        CAP,
        "the at-cap merge lands exactly on the cap"
    );

    let mut over = StateBuilder::with_cap(CAP);
    let err = over
        .merge(
            KEY,
            Value::String("x".repeat(at_cap_payload + 1)),
            "uploader",
        )
        .expect_err("one byte over the cap fails closed");
    assert_eq!(
        err.code, "state_cap_exceeded",
        "the over-cap error names the limit"
    );
    assert_eq!(
        err.category,
        ErrorCategory::RunFailure,
        "an over-cap merge is a run failure"
    );
    assert_eq!(
        err.task.as_deref(),
        Some("uploader"),
        "the error names the offending task"
    );
    assert_eq!(
        over.size_bytes(),
        2,
        "a rejected merge leaves the state untouched"
    );
}

#[test]
fn state_size_default_cap_is_the_named_constant() {
    // The boundary trio above runs against a narrowed cap; this pins the default cap to the named
    // `STATE_SIZE_MAX_BYTES` ceiling and proves a configured cap can only *narrow* it.
    assert_eq!(
        StateBuilder::new().cap_bytes(),
        STATE_SIZE_MAX_BYTES,
        "the default cap is the named 512-MiB ceiling"
    );
    assert_eq!(
        StateBuilder::with_cap(STATE_SIZE_MAX_BYTES + 1).cap_bytes(),
        STATE_SIZE_MAX_BYTES,
        "an over-ceiling configured cap is clamped down, never widened"
    );
}

// ---------------------------------------------------------------------------------------------
// JSON_DEPTH_MAX — via the state merge's depth guard.
// ---------------------------------------------------------------------------------------------

/// A value that is `arrays` nested single-element arrays around a scalar leaf; its own value-depth
/// is `arrays + 1`.
fn nested_arrays(arrays: u32) -> Value {
    let mut value = Value::Number(0.into());
    for _ in 0..arrays {
        value = Value::Array(vec![value]);
    }
    value
}

#[test]
fn json_depth_boundary_below_at_above() {
    // `output` nests one level below the top-level state object, so a value of its own depth
    // `JSON_DEPTH_MAX - 1` sits exactly at the state depth cap. In array counts that is
    // `JSON_DEPTH_MAX - 2` nested arrays (accepted); one deeper is rejected.
    let at_arrays = JSON_DEPTH_MAX - 2;

    let mut below = StateBuilder::new();
    below
        .merge("d", nested_arrays(at_arrays - 1), "nester")
        .unwrap_or_else(|_| panic!("one below the depth cap is accepted"));

    let mut at = StateBuilder::new();
    at.merge("d", nested_arrays(at_arrays), "nester")
        .unwrap_or_else(|_| panic!("at the depth cap is accepted"));

    let mut over = StateBuilder::new();
    let err = over
        .merge("d", nested_arrays(at_arrays + 1), "nester")
        .expect_err("one over the depth cap fails closed");
    assert_eq!(
        err.code, "json_too_deep",
        "the over-deep error names the limit"
    );
    assert_eq!(
        err.category,
        ErrorCategory::Validation,
        "an over-deep document is a validation error"
    );
    assert_eq!(
        over.size_bytes(),
        2,
        "a depth-rejected merge leaves the state untouched"
    );
}

// ---------------------------------------------------------------------------------------------
// FLOW_DEPTH_MAX — via a `flow`-import task dispatched at a given recursion depth.
// ---------------------------------------------------------------------------------------------

/// Run a single-`flow`-task parent at `depth` over the fakes, with the referenced child seeded. The
/// child is a plain `exec` sub-flow, so it consumes exactly one more recursion level.
fn run_flow_task_at_depth(depth: u32) -> (RunStatus, Option<String>) {
    let mut bundle = Bundle::new();
    bundle
        .process
        .push_result(Ok(tmx_conformance::stdout(b"leaf")));
    bundle.refs = tmx_testkit::FakeReferenceResolver::new().with_reference(
        "child",
        "child.yaml",
        tmx_core::ports::driven::SourceKind::Yaml,
    );
    bundle.loader = tmx_testkit::FakeSourceLoader::new().with_source(
        "child.yaml",
        json!({
            "name": "child",
            "tasks": [ { "name": "leaf", "type": "exec", "with": { "command": "echo leaf" } } ]
        }),
    );
    let parent = resolve_flow(json!({
        "name": "parent",
        "tasks": [ { "name": "sub", "type": "flow", "with": { "use": "child" } } ]
    }))
    .unwrap_or_else(|_| panic!("the parent flow resolves"));

    let id = run_id();
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let scheduler = SerialScheduler::new();
    let outcome = block_on_ready(runner.run(
        &id,
        &parent,
        &json!({}),
        bundle.ports(),
        &scheduler,
        &mut masker,
        &mut secrets,
        None,
        depth,
    ))
    .unwrap_or_else(|_| {
        panic!("the run itself completes (a depth overflow is recorded, not returned)")
    });
    let code = outcome.pipeline.results[0]
        .error
        .as_ref()
        .map(|e| e.code.to_string());
    (outcome.pipeline.status, code)
}

#[test]
fn flow_depth_boundary_below_at_above() {
    // A `flow` task at depth `d` recurses to level `d + 1`; the guard allows it while
    // `d + 1 <= FLOW_DEPTH_MAX`, i.e. the last depth a flow task may sit at is `FLOW_DEPTH_MAX - 1`.
    let at = FLOW_DEPTH_MAX - 1;

    let (below_status, below_code) = run_flow_task_at_depth(at - 1);
    assert_eq!(
        below_status,
        RunStatus::Ok,
        "one level below the bound recurses cleanly"
    );
    assert_eq!(below_code, None, "no error is recorded below the bound");

    let (at_status, at_code) = run_flow_task_at_depth(at);
    assert_eq!(
        at_status,
        RunStatus::Ok,
        "at the last allowed level the flow task still runs"
    );
    assert_eq!(at_code, None, "no error at the boundary");

    let (over_status, over_code) = run_flow_task_at_depth(FLOW_DEPTH_MAX);
    assert_eq!(
        over_status,
        RunStatus::Failed,
        "one level over the bound fails closed"
    );
    assert_eq!(
        over_code.as_deref(),
        Some("flow_depth_exceeded"),
        "the over-depth error names the recursion limit"
    );
}

// ---------------------------------------------------------------------------------------------
// TASKS_PER_FLOW_MAX — via the runner's pre-loop task-count guard.
// ---------------------------------------------------------------------------------------------

/// Run a flow of `n` trivially-passing `assert` tasks over the fakes; returns the terminal status,
/// or the typed error the pre-flight guard raised.
fn run_n_assert_tasks(n: u32) -> Result<RunStatus, tmx_core::RunError> {
    let tasks: Vec<Value> = (0..n)
        .map(|i| {
            json!({
                "name": format!("t{i}"),
                "type": "assert",
                "with": { "assertions": [ { "actual": true, "matcher": "toBeTruthy" } ] }
            })
        })
        .collect();
    let flow = resolve_flow(json!({ "name": "many", "tasks": tasks }))
        .unwrap_or_else(|_| panic!("the flow resolves"));
    let bundle = Bundle::new();
    let id = run_id();
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let scheduler = SerialScheduler::new();
    block_on_ready(runner.run(
        &id,
        &flow,
        &json!({}),
        bundle.ports(),
        &scheduler,
        &mut masker,
        &mut secrets,
        None,
        0,
    ))
    .map(|outcome| outcome.pipeline.status)
}

#[test]
fn tasks_per_flow_boundary_below_at_above() {
    let below = run_n_assert_tasks(TASKS_PER_FLOW_MAX - 1)
        .unwrap_or_else(|_| panic!("one below the task-count cap runs"));
    assert_eq!(
        below,
        RunStatus::Ok,
        "one below the task cap runs to completion"
    );

    let at = run_n_assert_tasks(TASKS_PER_FLOW_MAX)
        .unwrap_or_else(|_| panic!("exactly at the task-count cap runs"));
    assert_eq!(
        at,
        RunStatus::Ok,
        "exactly at the task cap runs to completion"
    );

    let err = run_n_assert_tasks(TASKS_PER_FLOW_MAX + 1)
        .expect_err("one over the task-count cap fails closed");
    assert_eq!(
        err.code, "too_many_tasks",
        "the over-count error names the limit"
    );
    assert_eq!(
        err.category,
        ErrorCategory::Validation,
        "too many tasks is a validation error"
    );
}

// ---------------------------------------------------------------------------------------------
// FANOUT_WIDTH_MAX — via the `--matrix` cross-product width guard (shared with map/eval fan-out).
// ---------------------------------------------------------------------------------------------

/// A single-axis matrix whose cross-product width equals `width`.
fn matrix_of_width(width: u32) -> IndexMap<String, Vec<Value>> {
    let mut axes: IndexMap<String, Vec<Value>> = IndexMap::new();
    axes.insert("a".to_string(), (0..width).map(|i| json!(i)).collect());
    axes
}

#[test]
fn fanout_width_boundary_below_at_above() {
    let below = matrix_combinations(&matrix_of_width(FANOUT_WIDTH_MAX - 1))
        .unwrap_or_else(|_| panic!("one below the fan-out width builds"));
    assert_eq!(
        below.len() as u64,
        u64::from(FANOUT_WIDTH_MAX - 1),
        "one below the width limit yields that many combinations"
    );

    let at = matrix_combinations(&matrix_of_width(FANOUT_WIDTH_MAX))
        .unwrap_or_else(|_| panic!("exactly at the fan-out width builds"));
    assert_eq!(
        at.len() as u64,
        u64::from(FANOUT_WIDTH_MAX),
        "exactly at the width limit is still allowed"
    );

    let err = matrix_combinations(&matrix_of_width(FANOUT_WIDTH_MAX + 1))
        .expect_err("one over the fan-out width fails closed");
    assert_eq!(
        err.code, "fanout_too_wide",
        "the over-width error names the limit"
    );
}

// ---------------------------------------------------------------------------------------------
// EXPR_LEN_MAX_BYTES / EXPR_DEPTH_MAX — via the interpolation expression evaluator.
// ---------------------------------------------------------------------------------------------

#[test]
fn expr_length_boundary_below_at_above() {
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    let max = EXPR_LEN_MAX_BYTES as usize;
    // A string literal of exact byte length `len`: a quote, `len - 2` filler chars, a quote.
    let expr_of_len = |len: usize| {
        let mut s = String::with_capacity(len);
        s.push('"');
        s.extend(std::iter::repeat_n('a', len - 2));
        s.push('"');
        s
    };

    assert!(
        evaluate(&expr_of_len(max - 1), &scope).is_ok(),
        "one below the length limit parses"
    );
    assert!(
        evaluate(&expr_of_len(max), &scope).is_ok(),
        "exactly at the length limit is still accepted"
    );
    let err = evaluate(&expr_of_len(max + 1), &scope)
        .expect_err("one over the length limit fails closed");
    assert_eq!(
        err.code, "expr_too_long",
        "the over-length error names the limit"
    );
    assert_eq!(
        err.category,
        ErrorCategory::Resolution,
        "an over-long expression is a resolution error"
    );
}

#[test]
fn expr_depth_boundary_below_at_above() {
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    // AST depth of `("(" * p) 1 (")" * p)` is `p + 1`, so `p = depth - 1`.
    let expr_of_depth = |depth: u32| {
        let p = (depth - 1) as usize;
        format!("{}1{}", "(".repeat(p), ")".repeat(p))
    };
    let max = EXPR_DEPTH_MAX;

    assert!(
        evaluate(&expr_of_depth(max - 1), &scope).is_ok(),
        "one below the depth limit parses"
    );
    assert!(
        evaluate(&expr_of_depth(max), &scope).is_ok(),
        "exactly at the depth limit is still accepted"
    );
    let err = evaluate(&expr_of_depth(max + 1), &scope)
        .expect_err("one over the depth limit fails closed");
    assert_eq!(
        err.code, "expr_too_deep",
        "the over-deep error names the limit"
    );
    assert_eq!(
        err.category,
        ErrorCategory::Resolution,
        "an over-deep expression is a resolution error"
    );
}
