//! Golden-Flow conformance tier (Task 32, O1/O4).
//!
//! Each case drives a Flow through [`RunFlow`](tmx_core::ports::driving::RunFlow) — the real
//! `EngineRunFlow` use case — over the deterministic `tmx-testkit` fakes, then asserts the recorded
//! event stream *and* the final state against a golden value. Coverage spans every task type
//! (`exec`, `assert`, `fetch`, `file`, `store`, `chat-completion`, `map`, `eval`, `flow` import, and
//! the `if` skip) and every lifecycle hook (`create`/`change`/`destroy`/`error`).
//!
//! `map` and `eval` are the engine's fan-out task types; the sequential `EngineRunFlow` delegates
//! them to [`run_map`]/[`run_eval`] (the same functions the CLI drives at the top level), so those
//! two cases exercise the fan-out path directly over the `SerialScheduler` — still the same recorded
//! fakes, still deterministic.
//!
//! Determinism is asserted structurally: [`golden`] runs each Flow *twice* over two fresh bundles
//! and requires byte-identical event streams (as NDJSON), byte-identical final state, and identical
//! run ids — the byte-for-byte reproducibility O1 names, sourced only from the fixed
//! `Clock`/`IdGenerator`/`SerialScheduler` fakes.

use serde_json::{Value, json};

use tmx_conformance::{
    Bundle, block_on_ready, empty_scope, event_tags, hook_sequence, run_engine, stdout,
};
use tmx_core::ports::driven::ChatResponse;
use tmx_core::{Milliseconds, RunRecord, RunStatus, TaskStatus, run_eval, run_map};
use tmx_schema::limits::CONCURRENCY_MAX;
use tmx_schema::task::{EvalWith, MapWith};
use tmx_testkit::{FakeChatModel, RecordingProcessRunner, SerialScheduler};

/// Drive `flow` at `reference` twice over two fresh, identically-scripted bundles, asserting the two
/// runs are byte-for-byte identical (event stream + final state + run id) — the determinism spine of
/// the tier — and return the (first) terminal record for the caller's per-case golden assertions.
///
/// `script` seeds the recording/builder fakes (process results, HTTP responses, seeded files, …)
/// before the Flow is seeded and run; it must be a pure function of the bundle so both runs match.
type HookSequence = Vec<(&'static str, String)>;

#[allow(clippy::expect_used)] // a fixture/harness helper (not a `#[test]`): an expect names the harness fault
fn golden(
    reference: &str,
    flow: Value,
    inputs: Value,
    script: impl Fn(&mut Bundle),
) -> (RunRecord, Vec<String>, HookSequence) {
    let run_once = || {
        let mut bundle = Bundle::new();
        // Seed the entry Flow first, then let the script script the recording fakes (and, for a
        // multi-source case like a `flow` import, re-seed the loader with every referenced source).
        bundle.seed_flow(reference, flow.clone());
        script(&mut bundle);
        let record =
            run_engine(&bundle, reference, inputs.clone()).expect("the golden run completes");
        let events = bundle.events.events();
        let ndjson = bundle.events.ndjson().expect("the event stream serialises");
        (record, event_tags(&events), ndjson, hook_sequence(&events))
    };

    let (record_a, tags_a, ndjson_a, hooks_a) = run_once();
    let (record_b, _tags_b, ndjson_b, hooks_b) = run_once();

    assert_eq!(
        ndjson_a, ndjson_b,
        "two runs of {reference:?} emit byte-identical event streams"
    );
    assert_eq!(
        record_a.final_state, record_b.final_state,
        "two runs of {reference:?} return byte-identical final state"
    );
    assert_eq!(
        record_a.id, record_b.id,
        "two runs of {reference:?} mint the identical run id from the seeded generator"
    );
    assert_eq!(hooks_a, hooks_b, "the hook sequence is reproducible");
    (record_a, tags_a, hooks_a)
}

/// The final state of a completed record as a JSON value (panicking only on a harness bug: a
/// completed run always carries a final state).
fn final_state(record: &RunRecord) -> Value {
    record
        .final_state
        .as_ref()
        .map(|state| state.as_value().clone())
        .unwrap_or_else(|| panic!("a completed run must carry a final state"))
}

#[test]
fn golden_exec_and_assert_with_create_and_destroy_hooks() {
    // The flagship golden Flow — the reviewable one (O4). An `exec` produces output, an `assert`
    // reads it via `${{ tasks.* }}`, and the context declares create + destroy hooks (each a single
    // `exec` body). The asserted event stream is exactly the spec lifecycle: run.start → create hook
    // → per-task → destroy hook → run.finish. A hook body runs through the same runner, so its own
    // `exec` emits a bracketed `task.start`/`task.finish` inside the `hook.start`/`hook.finish` pair.
    let (record, tags, _hooks) = golden(
        "deploy",
        json!({
            "name": "deploy",
            "context": { "hooks": {
                "create":  [ { "name": "on_create",  "type": "exec", "with": { "command": "echo c" } } ],
                "destroy": [ { "name": "on_destroy", "type": "exec", "with": { "command": "echo d" } } ]
            } },
            "tasks": [
                { "name": "build", "type": "exec", "with": { "command": "echo built-ok" } },
                {
                    "name": "check",
                    "type": "assert",
                    "with": { "assertions": [
                        { "actual": "${{ tasks.build.message }}", "matcher": "toBe", "expected": "built-ok" }
                    ] }
                }
            ]
        }),
        json!({}),
        |bundle| {
            // Dispatch order of the three execs: on_create, build, on_destroy. The assert runs none.
            for line in ["created", "built-ok", "destroyed"] {
                bundle.process.push_result(Ok(stdout(line.as_bytes())));
            }
        },
    );

    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the clean run reaches terminal ok"
    );
    assert_eq!(
        final_state(&record),
        json!({
            "build": { "message": "built-ok" },
            "check": { "passed": true, "assertions": 1 }
        }),
        "the merged final state matches the golden value (hook outputs never merge into pipeline state)"
    );
    // The canonical lifecycle stream, hook bodies bracketing their own task events.
    assert_eq!(
        tags,
        vec![
            "run.start",
            "hook.start",
            "task.start",
            "task.finish",
            "hook.finish", // create (on_create exec)
            "task.start",
            "task.finish", // build
            "task.start",
            "task.finish", // check (assert)
            "hook.start",
            "task.start",
            "task.finish",
            "hook.finish", // destroy (on_destroy exec)
            "run.finish",
        ],
        "the event stream is exactly the spec lifecycle create -> per-task -> destroy"
    );
}

#[test]
fn golden_change_hook_fires_once_per_state_changing_task() {
    // Lifecycle `change` coverage: two `exec` tasks each change state, firing `change` once apiece; a
    // third, `if`-gated-off task is skipped and fires no `change`. Asserted via the (robust) filtered
    // hook sequence rather than the full interleaved stream.
    let (record, _tags, hooks) = golden(
        "changes",
        json!({
            "name": "changes",
            "inputs": { "enabled": { "default": false } },
            "context": { "hooks": {
                "change": [ { "name": "on_change", "type": "exec", "with": { "command": "echo x" } } ]
            } },
            "tasks": [
                { "name": "alpha", "type": "exec", "with": { "command": "echo a" } },
                { "name": "beta",  "type": "exec", "with": { "command": "echo b" } },
                { "name": "gamma", "if": "${{ inputs.enabled }}", "type": "exec", "with": { "command": "echo g" } }
            ]
        }),
        json!({}),
        |bundle| {
            // Dispatch order: alpha, on_change(alpha), beta, on_change(beta). gamma is skipped.
            for line in ["a", "xa", "b", "xb"] {
                bundle.process.push_result(Ok(stdout(line.as_bytes())));
            }
        },
    );

    assert_eq!(record.status, RunStatus::Ok, "the run succeeds");
    // gamma was skipped (not a state change), so exactly two `change` fires — alpha and beta.
    assert_eq!(
        hooks,
        vec![
            ("start", "change".to_string()),
            ("finish", "change".to_string()),
            ("start", "change".to_string()),
            ("finish", "change".to_string()),
        ],
        "change fires once per state-changing task and never for the skipped gamma"
    );
    assert_eq!(
        record
            .results
            .iter()
            .filter(|r| r.status == TaskStatus::Skipped)
            .count(),
        1,
        "the gated gamma task was skipped"
    );
}

#[test]
fn golden_error_hook_fires_on_a_failing_run() {
    // Lifecycle `error` coverage: a failing assert aborts the Pipeline, firing error then destroy.
    // Asserted via the filtered hook sequence (robust to the hook bodies' own task events).
    let (record, _tags, hooks) = golden(
        "gated-run",
        json!({
            "name": "gated-run",
            "context": { "hooks": {
                "create":  [ { "name": "on_create",  "type": "exec", "with": { "command": "echo c" } } ],
                "error":   [ { "name": "on_error",   "type": "exec", "with": { "command": "echo e" } } ],
                "destroy": [ { "name": "on_destroy", "type": "exec", "with": { "command": "echo d" } } ]
            } },
            "tasks": [ {
                "name": "gate",
                "type": "assert",
                "with": { "assertions": [ { "actual": 1, "matcher": "toBe", "expected": 2 } ] }
            } ]
        }),
        json!({}),
        |bundle| {
            for line in ["c", "e", "d"] {
                bundle.process.push_result(Ok(stdout(line.as_bytes())));
            }
        },
    );

    assert_eq!(
        record.status,
        RunStatus::Failed,
        "the failing assert fails the run"
    );
    assert_eq!(
        record.results[0].error.as_ref().map(|e| e.code),
        Some("assertion_failed"),
        "the recorded error is the failed assertion"
    );
    // create fires on entry; error fires on the abort; destroy is the finally — in that order.
    assert_eq!(
        hooks,
        vec![
            ("start", "create".to_string()),
            ("finish", "create".to_string()),
            ("start", "error".to_string()),
            ("finish", "error".to_string()),
            ("start", "destroy".to_string()),
            ("finish", "destroy".to_string()),
        ],
        "a failure fires create -> error -> destroy in lifecycle order"
    );
}

#[test]
fn golden_if_skip_leaves_state_untouched() {
    // The `if` gate: a falsy condition emits task.skip and runs no effect.
    let (record, tags, _hooks) = golden(
        "maybe",
        json!({
            "name": "maybe",
            "inputs": { "enabled": { "default": false } },
            "tasks": [
                { "name": "guarded", "if": "${{ inputs.enabled }}", "type": "exec", "with": { "command": "echo x" } }
            ]
        }),
        json!({}),
        |_bundle| {},
    );

    assert_eq!(
        record.status,
        RunStatus::Ok,
        "a skipped task is not a failure"
    );
    assert_eq!(
        record.results[0].status,
        TaskStatus::Skipped,
        "the task was skipped"
    );
    assert_eq!(
        final_state(&record),
        json!({}),
        "a skip leaves the state empty"
    );
    assert_eq!(
        tags,
        vec!["run.start", "task.skip", "run.finish"],
        "a skip emits task.skip, not start/finish"
    );
}

#[test]
fn golden_fetch_merges_the_response_body() {
    // `fetch`: the seeded HTTP body normalises to `{ message }` and merges under the task name.
    let (record, tags, _hooks) = golden(
        "grab",
        json!({
            "name": "grab",
            "tasks": [
                { "name": "page", "type": "fetch", "with": { "url": "https://example.test/data" } }
            ]
        }),
        json!({}),
        |bundle| {
            bundle.http =
                tmx_testkit::FakeHttpClient::new().with_response(200, b"payload-body".to_vec());
        },
    );

    assert_eq!(record.status, RunStatus::Ok, "the fetch run succeeds");
    assert_eq!(
        final_state(&record),
        json!({ "page": { "message": "payload-body" } }),
        "the fetched body normalises into state under the task name"
    );
    assert_eq!(
        tags,
        vec!["run.start", "task.start", "task.finish", "run.finish"],
        "the fetch stream is a single bracketed task"
    );
}

#[test]
fn golden_file_write_then_read_round_trips_through_memory() {
    // `file`: write then read the same path; write yields `{ ok: true }`, read normalises to
    // `{ message }`. Both operations cross the same in-memory FileSystem port.
    let (record, _tags, _hooks) = golden(
        "files",
        json!({
            "name": "files",
            "tasks": [
                { "name": "save", "type": "file", "with": { "operation": "write", "path": "/out.txt", "content": "written-bytes" } },
                { "name": "load", "type": "file", "with": { "operation": "read", "path": "/out.txt" } }
            ]
        }),
        json!({}),
        |_bundle| {},
    );

    assert_eq!(record.status, RunStatus::Ok, "the file round-trip succeeds");
    assert_eq!(
        final_state(&record),
        json!({
            "save": { "ok": true },
            "load": { "message": "written-bytes" }
        }),
        "the write reports ok and the read returns the written bytes"
    );
}

#[test]
fn golden_store_put_then_get_round_trips_through_memory() {
    // `store`: put then get the same key; put yields `{ ok: true }`, get normalises to `{ message }`.
    let (record, _tags, _hooks) = golden(
        "objects",
        json!({
            "name": "objects",
            "tasks": [
                { "name": "up",   "type": "store", "with": { "operation": "put", "bucket": "b", "key": "k", "content": "object-body" } },
                { "name": "down", "type": "store", "with": { "operation": "get", "bucket": "b", "key": "k" } }
            ]
        }),
        json!({}),
        |_bundle| {},
    );

    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the store round-trip succeeds"
    );
    assert_eq!(
        final_state(&record),
        json!({
            "up":   { "ok": true },
            "down": { "message": "object-body" }
        }),
        "the put reports ok and the get returns the stored bytes"
    );
}

#[test]
fn golden_chat_completion_merges_the_completion() {
    // `chat-completion`: the canned completion merges as `{ content, model }` under the task name.
    let (record, _tags, _hooks) = golden(
        "ask",
        json!({
            "name": "ask",
            "tasks": [
                {
                    "name": "reply",
                    "type": "chat-completion",
                    "with": { "model": "test-model", "messages": [ { "role": "user", "content": "hello" } ] }
                }
            ]
        }),
        json!({}),
        |bundle| {
            bundle.chat = FakeChatModel::new();
            bundle.chat.push_result(Ok(ChatResponse {
                content: "the-completion".to_string(),
                model: "test-model".to_string(),
                prompt_tokens: Some(7),
                completion_tokens: Some(3),
                ms: Milliseconds(0),
            }));
        },
    );

    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the chat-completion run succeeds"
    );
    assert_eq!(
        final_state(&record),
        json!({ "reply": { "content": "the-completion", "model": "test-model" } }),
        "the completion merges under the task name"
    );
}

#[test]
fn golden_flow_import_merges_the_sub_flows_state() {
    // `flow` import: a parent task runs a referenced sub-flow one level deep and merges its final
    // state under the task's name.
    let (record, _tags, _hooks) = golden(
        "parent",
        json!({
            "name": "parent",
            "tasks": [ { "name": "sub", "type": "flow", "with": { "use": "child" } } ]
        }),
        json!({}),
        |bundle| {
            bundle.process.push_result(Ok(stdout(b"inner")));
            // The parent + child are both seeded through the loader; seed_flow (called after this
            // script) overwrites refs/loader with only the parent, so seed both here and re-seed the
            // parent to keep the child reachable.
            bundle.refs = tmx_testkit::FakeReferenceResolver::new()
                .with_reference(
                    "parent",
                    "parent.yaml",
                    tmx_core::ports::driven::SourceKind::Yaml,
                )
                .with_reference(
                    "child",
                    "child.yaml",
                    tmx_core::ports::driven::SourceKind::Yaml,
                );
            bundle.loader = tmx_testkit::FakeSourceLoader::new()
                .with_source(
                    "parent.yaml",
                    json!({
                        "name": "parent",
                        "tasks": [ { "name": "sub", "type": "flow", "with": { "use": "child" } } ]
                    }),
                )
                .with_source(
                    "child.yaml",
                    json!({
                        "name": "child",
                        "tasks": [ { "name": "leaf", "type": "exec", "with": { "command": "echo inner" } } ]
                    }),
                );
        },
    );

    assert_eq!(record.status, RunStatus::Ok, "the nested run succeeds");
    assert_eq!(
        final_state(&record),
        json!({ "sub": { "leaf": { "message": "inner" } } }),
        "the sub-flow's final state merges under the flow task's name"
    );
}

#[test]
fn golden_map_fans_out_in_item_order() {
    // `map`: the fan-out task type, driven over the SerialScheduler exactly as the CLI drives a
    // top-level map. The output array follows item order regardless of the (serial) completion order,
    // and the width stays within the fan-out bound.
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    let map: MapWith = serde_json::from_value(json!({
        "items": [ { "v": 0 }, { "v": 1 }, { "v": 2 }, { "v": 3 } ],
        "concurrency": 2,
        "task": { "type": "exec", "with": { "command": "square" } }
    }))
    .unwrap_or_else(|error| panic!("valid MapWith fixture: {error}"));

    let run_twice = || {
        block_on_ready(run_map(
            &map,
            "squares",
            &scope,
            &SerialScheduler::new(),
            CONCURRENCY_MAX,
            0,
            |_index, item, _depth| async move {
                // The map binds each element under `item` (with `.index`); read its declared value.
                let n = item.get("v").and_then(Value::as_u64).unwrap_or(0);
                Ok::<Value, tmx_core::RunError>(json!({ "n": n * n }))
            },
        ))
        .unwrap_or_else(|error| panic!("the map fan-out completes: {error}"))
    };

    let first = run_twice();
    let second = run_twice();
    assert_eq!(
        first, second,
        "two map fan-outs produce byte-identical output"
    );
    assert_eq!(
        first,
        json!([{ "n": 0 }, { "n": 1 }, { "n": 4 }, { "n": 9 }]),
        "the output array follows item (index) order, not completion order"
    );
}

#[test]
fn golden_eval_emits_a_scorecard_and_gates_on_its_threshold() {
    // `eval`: the measured fan-out task type. A single matcher scorer over one synthetic case emits a
    // full scorecard; a threshold at/below the achieved metric passes.
    let empty = Value::Object(serde_json::Map::new());
    let scope = empty_scope(&empty);
    let eval: EvalWith = serde_json::from_value(json!({
        "subject": { "type": "exec", "with": { "command": "echo" } },
        "scorers": [ { "name": "truthy", "type": "matcher", "matcher": "toBeTruthy" } ],
        "threshold": { "metric": "mean", "min": 0.5 }
    }))
    .unwrap_or_else(|error| panic!("valid EvalWith fixture: {error}"));

    let run_twice = || {
        block_on_ready(run_eval(
            &eval,
            "quality",
            &scope,
            &SerialScheduler::new(),
            &FakeChatModel::new(),
            &RecordingProcessRunner::new(),
            CONCURRENCY_MAX,
            0,
            |_index, _case, _depth| async move { Ok::<Value, tmx_core::RunError>(json!("non-empty")) },
        ))
        .unwrap_or_else(|error| panic!("the eval completes and meets its threshold: {error}"))
    };

    let first = run_twice();
    let second = run_twice();
    assert_eq!(first, second, "two evals produce byte-identical scorecards");
    assert_eq!(
        first["passed"],
        json!(true),
        "the achieved mean clears the threshold"
    );
    assert_eq!(
        first["summary"]["mean"],
        json!(1.0),
        "the single truthy case scores a perfect mean"
    );
    assert_eq!(
        first["cases"].as_array().map(Vec::len),
        Some(1),
        "one scorecard entry per case"
    );
}
