//! Negative-space conformance tier (Task 32, O2).
//!
//! The four required fail-closed cases the guidelines mandate, each asserted end to end over the
//! deterministic fakes: a **leaked secret** is redacted out of both the final state and the event
//! stream; an **over-cap state** aborts with the typed `state_cap_exceeded` naming the task; a
//! **too-deep recursion** is `flow_depth_exceeded`; and a **duplicate task name** is rejected before
//! any task runs. None is a panic, a silent truncation, or an unmasked leak.

use serde_json::json;

use tmx_conformance::{
    Bundle, block_on_ready, event_tags, run_engine, run_engine_with_config, stdout,
};
use tmx_core::{ErrorCategory, Masker, PipelineRunner, RunConfig, RunId, RunStatus, resolve_flow};

const A_RUN_ID: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

#[test]
fn a_requested_secret_is_never_leaked_in_state_or_the_event_stream() {
    // A task requests a secret and echoes it; neither the returned final state nor any emitted event
    // may carry the raw value — the Masker redacts it at both boundaries.
    let mut bundle = Bundle::new();
    bundle.seed_flow(
        "leaky",
        json!({
            "name": "leaky",
            "context": { "secrets": { "TOKEN": { "env": "TOKEN_ENV" } } },
            "tasks": [
                { "name": "leak", "type": "exec", "secrets": ["TOKEN"], "with": { "command": "echo $TOKEN" } }
            ]
        }),
    );
    bundle.secrets =
        tmx_testkit::FakeSecretResolver::new().with_secret("TOKEN_ENV", "supersecretvalue");
    bundle
        .process
        .push_result(Ok(stdout(b"supersecretvalue leaked")));

    let record = run_engine(&bundle, "leaky", json!({})).expect("the run completes");
    let state = record
        .final_state
        .as_ref()
        .map(|s| s.as_value().clone())
        .unwrap_or_else(|| panic!("a completed run carries a final state"));
    assert_eq!(
        state["leak"]["message"],
        json!("[REDACTED] leaked"),
        "the secret is redacted out of the final state"
    );

    let stream = bundle.events.ndjson().expect("the event stream serialises");
    assert!(
        !stream.contains("supersecretvalue"),
        "no emitted event leaks the raw secret value"
    );
    // And the redaction marker is actually present — the leak was scrubbed, not merely absent.
    assert!(
        stream.contains("[REDACTED]"),
        "the emitted stream carries the redaction marker where the secret was"
    );
}

#[test]
fn an_over_cap_state_aborts_with_a_typed_error_naming_the_task() {
    // A task whose output would grow the state past a narrowed cap aborts the run with the typed
    // `state_cap_exceeded` naming the task — never a silent truncation of the output.
    let mut bundle = Bundle::new();
    bundle.seed_flow(
        "grow",
        json!({
            "name": "grow",
            "tasks": [ { "name": "uploader", "type": "exec", "with": { "command": "cat big" } } ]
        }),
    );
    // 512 bytes of output cannot fit under a 64-byte state cap.
    bundle.process.push_result(Ok(stdout(&vec![b'x'; 512])));
    let config = RunConfig {
        max_state_size_bytes: Some(64),
        ..RunConfig::default()
    };

    let record =
        run_engine_with_config(&bundle, "grow", json!({}), config).expect("the run completes");
    assert_eq!(
        record.status,
        RunStatus::Failed,
        "an over-cap merge fails the run closed"
    );
    let error = record.results[0]
        .error
        .as_ref()
        .unwrap_or_else(|| panic!("the failing task recorded an error"));
    assert_eq!(
        error.code, "state_cap_exceeded",
        "the abort names the state-size limit"
    );
    assert_eq!(
        error.category,
        ErrorCategory::RunFailure,
        "an over-cap merge is a run failure"
    );
    assert_eq!(
        error.task.as_deref(),
        Some("uploader"),
        "the error names the offending task"
    );
}

#[test]
fn a_too_deep_flow_recursion_is_flow_depth_exceeded() {
    // A `flow` task dispatched at the recursion ceiling fails closed *before* any load — the depth
    // guard trips first, so the reference resolver is never consulted.
    let bundle = Bundle::new();
    let flow = resolve_flow(json!({
        "name": "nest",
        "tasks": [ { "name": "inner", "type": "flow", "with": { "use": "unseeded" } } ]
    }))
    .expect("the flow resolves");

    let id = RunId::new(A_RUN_ID).expect("valid id");
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    // Start at the ceiling: the single flow task's guard sees depth + 1 > FLOW_DEPTH_MAX.
    let ceiling = tmx_schema::limits::FLOW_DEPTH_MAX;
    let outcome = block_on_ready(runner.run(
        &id,
        &flow,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        None,
        ceiling,
    ))
    .expect("the run itself completes — the depth overflow is recorded, not returned");

    assert_eq!(
        outcome.pipeline.status,
        RunStatus::Failed,
        "a too-deep nest fails the run"
    );
    let error = outcome.pipeline.results[0]
        .error
        .as_ref()
        .expect("the flow task recorded an error");
    assert_eq!(
        error.code, "flow_depth_exceeded",
        "the error names the recursion limit"
    );
    assert_eq!(
        error.category,
        ErrorCategory::Resolution,
        "a depth overflow is a resolution failure"
    );
    // The guard tripped before any load: the single flow task starts then errors, and no sub-flow
    // ran (a sub-flow would have driven its own tasks into the stream). The reference to the
    // "unseeded" child was never resolved, so the run fails on the guard, not on a load error.
    assert_eq!(
        event_tags(&bundle.events.events()),
        vec!["run.start", "task.start", "task.error", "run.finish"],
        "the depth guard aborts the single task before any sub-flow load"
    );
}

#[test]
fn a_duplicate_task_name_is_rejected_before_any_task_runs() {
    // Two tasks share a name; the pre-flight guard rejects the flow with `duplicate_task_name` before
    // the loop — a hard `Err`, never a silently-last-wins merge.
    let bundle = Bundle::new();
    let flow = resolve_flow(json!({
        "name": "dupes",
        "tasks": [
            { "name": "dup", "type": "exec", "with": { "command": "a" } },
            { "name": "dup", "type": "exec", "with": { "command": "b" } }
        ]
    }))
    .expect("the flow resolves");

    let id = RunId::new(A_RUN_ID).expect("valid id");
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let err = block_on_ready(runner.run(
        &id,
        &flow,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        None,
        0,
    ))
    .expect_err("a duplicate task name is rejected");

    assert_eq!(
        err.code, "duplicate_task_name",
        "the rejection names the fault"
    );
    assert_eq!(
        err.category,
        ErrorCategory::Validation,
        "a bad name set is a validation failure"
    );
    // Rejected before the loop: nothing was dispatched, so no event was ever emitted.
    assert!(
        bundle.events.events().is_empty(),
        "the pre-flight rejection emitted no events"
    );
}
