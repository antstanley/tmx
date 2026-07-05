//! Serialize-and-validate: every runtime type's `Serialize` output validates against its `$def` in
//! [`.specs/canonical-types.schema.json`](../../../.specs/canonical-types.schema.json).
//!
//! This is the reviewable evidence for task 04. The sidecar schema is the authoritative shape of what
//! the engine emits; this test runs each `tmx-core` runtime value through its real `Serialize` impl
//! and validates the JSON against the matching `$def` with a real JSON-Schema validator. If a field
//! were mis-named (a `camelCase` slip), a wrong integer width leaked, or an unexpected key appeared,
//! the `additionalProperties:false` / typed `$def` would reject it here.
//!
//! Negative space is proven too: an out-of-enum status string and an extra property are each rejected
//! by the compiled validator, so the validator is shown to actually discriminate — a green result is
//! not vacuous.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use tmx_core::{
    BlobWrapper, Diagnostic, ErrorCategory, EvalCase, EvalSummary, Event, MessageWrapper,
    Milliseconds, PipelineState, RunError, RunId, RunRecord, RunStatus, Scorecard, Severity,
    TaskResult, TaskStatus, Timestamp,
};

/// A well-formed UUIDv7 used across the representative values.
const RUN_ID: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";
/// A well-formed RFC 3339 UTC instant.
const WHEN: &str = "2026-07-05T12:00:00Z";

/// Load the canonical-types sidecar schema, resolved from this crate's manifest dir
/// (`<root>/crates/tmx-core`) so the test is independent of the process working directory.
fn schema_document() -> Value {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = match manifest_dir.parent().and_then(Path::parent) {
        Some(root) => root.to_path_buf(),
        None => panic!(
            "workspace root is two levels above {}",
            manifest_dir.display()
        ),
    };
    let path = root.join(".specs").join("canonical-types.schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("the sidecar schema is valid JSON: {e}"))
}

/// Compile a validator that requires its instance to match `#/$defs/<def_name>` of the sidecar
/// schema. The schema's `$defs` are lifted into a fresh wrapper whose sole top-level keyword is the
/// `$ref`, so a non-object `$def` (e.g. `RunId`, a string) is not also forced through the sidecar's
/// top-level `type: object`.
fn validator_for_def(schema: &Value, def_name: &str) -> jsonschema::Validator {
    let defs = match schema.get("$defs").cloned() {
        Some(defs) => defs,
        None => panic!("the sidecar schema has a $defs object"),
    };
    let wrapper = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": defs,
        "$ref": format!("#/$defs/{def_name}"),
    });
    jsonschema::validator_for(&wrapper)
        .unwrap_or_else(|e| panic!("cannot compile validator for {def_name}: {e}"))
}

/// Assert `instance` validates against `#/$defs/<def_name>`, listing every schema error on failure.
fn assert_valid(schema: &Value, def_name: &str, instance: &Value) {
    let validator = validator_for_def(schema, def_name);
    if !validator.is_valid(instance) {
        let errors: Vec<String> = validator
            .iter_errors(instance)
            .map(|e| e.to_string())
            .collect();
        panic!(
            "{def_name} instance failed schema validation:\n  instance = {instance}\n  errors = {errors:#?}"
        );
    }
}

/// Assert `instance` does NOT validate against `#/$defs/<def_name>` — negative space proving the
/// validator discriminates.
fn assert_invalid(schema: &Value, def_name: &str, instance: &Value) {
    let validator = validator_for_def(schema, def_name);
    assert!(
        !validator.is_valid(instance),
        "{def_name} must REJECT this instance but accepted it: {instance}"
    );
}

/// Serialise `value` to JSON via its real `Serialize` impl, panicking with context on failure.
fn ser<T: serde::Serialize>(value: &T, what: &str) -> Value {
    serde_json::to_value(value).unwrap_or_else(|e| panic!("{what} must serialise: {e}"))
}

fn run_id() -> RunId {
    RunId::new(RUN_ID).unwrap_or_else(|e| panic!("the constant RunId is a valid UUIDv7: {e}"))
}

#[test]
fn every_runtime_def_has_a_type_that_validates() {
    let schema = schema_document();

    // Scalar / string / enum $defs.
    assert_valid(&schema, "RunId", &ser(&run_id(), "RunId"));
    assert_valid(
        &schema,
        "Milliseconds",
        &ser(&Milliseconds(1234), "Milliseconds"),
    );
    assert_valid(
        &schema,
        "Timestamp",
        &ser(&Timestamp::new(WHEN), "Timestamp"),
    );
    assert_valid(&schema, "RunStatus", &ser(&RunStatus::Ok, "RunStatus"));
    assert_valid(
        &schema,
        "TaskStatus",
        &ser(&TaskStatus::Skipped, "TaskStatus"),
    );
    assert_valid(
        &schema,
        "ErrorCategory",
        &ser(&ErrorCategory::Resolution, "ErrorCategory"),
    );

    // State and output-normalisation wrappers.
    let state = PipelineState::new(json!({ "build": { "ok": true } })).expect("object state");
    assert_valid(&schema, "PipelineState", &ser(&state, "PipelineState"));
    assert_valid(
        &schema,
        "PipelineState",
        &ser(&PipelineState::empty(), "empty PipelineState"),
    );
    assert_valid(
        &schema,
        "MessageWrapper",
        &ser(
            &MessageWrapper {
                message: "hi".into(),
            },
            "MessageWrapper",
        ),
    );
    assert_valid(
        &schema,
        "BlobWrapper",
        &ser(
            &BlobWrapper {
                blob: "aGVsbG8=".into(),
            },
            "BlobWrapper",
        ),
    );

    // The typed error and diagnostic.
    let err = RunError::run_failure("state_cap_exceeded", "state exceeded the cap")
        .with_task("upload")
        .with_path("/tasks/upload");
    assert_valid(&schema, "RunError", &ser(&err, "RunError"));
    let diag = Diagnostic::new(
        Severity::Warning,
        "undeclared_secret",
        "secret not declared",
    )
    .with_path("flow.yaml#/tasks/0");
    assert_valid(&schema, "Diagnostic", &ser(&diag, "Diagnostic"));

    // TaskResult in both the ok-with-output and error shapes.
    let ok_result = TaskResult {
        name: "build".into(),
        status: TaskStatus::Ok,
        output: Some(json!({ "artifact": "app.tar" })),
        error: None,
        started_at: Timestamp::new(WHEN),
        ms: Milliseconds(42),
    };
    assert_valid(&schema, "TaskResult", &ser(&ok_result, "ok TaskResult"));
    let err_result = TaskResult {
        name: "deploy".into(),
        status: TaskStatus::Error,
        output: None,
        error: Some(RunError::run_failure(
            "assert_failed",
            "deploy check failed",
        )),
        started_at: Timestamp::new(WHEN),
        ms: Milliseconds(7),
    };
    assert_valid(&schema, "TaskResult", &ser(&err_result, "error TaskResult"));

    // Eval types and the scorecard.
    let mut scores = indexmap::IndexMap::new();
    scores.insert("exact_match".to_string(), 1.0_f64);
    scores.insert("similarity".to_string(), 0.75_f64);
    let case = EvalCase {
        case: Some(json!({ "prompt": "2+2" })),
        output: Some(json!("4")),
        scores,
        score: 0.875,
        passed: true,
    };
    assert_valid(&schema, "EvalCase", &ser(&case, "EvalCase"));
    let summary = EvalSummary {
        mean: 0.8,
        weighted_mean: 0.82,
        pass_rate: 0.9,
        min: Some(0.5),
        p50: Some(0.85),
        p90: Some(0.95),
        count: 10,
    };
    assert_valid(&schema, "EvalSummary", &ser(&summary, "EvalSummary"));
    let scorecard = Scorecard {
        cases: vec![case],
        summary,
        passed: true,
    };
    assert_valid(&schema, "Scorecard", &ser(&scorecard, "Scorecard"));

    // Every Event variant validates against the (flat, internally-tagged) Event $def.
    let events = [
        Event::RunStart {
            id: run_id(),
            flow: "deploy".into(),
        },
        Event::RunFinish {
            id: run_id(),
            status: RunStatus::Ok,
            ms: Milliseconds(1000),
        },
        Event::TaskStart {
            name: "build".into(),
        },
        Event::TaskFinish {
            name: "build".into(),
            status: TaskStatus::Ok,
            ms: Milliseconds(42),
            output: Some(json!({ "ok": true })),
        },
        Event::TaskSkip {
            name: "lint".into(),
            reason: "if=false".into(),
        },
        Event::TaskError {
            name: "deploy".into(),
            error: RunError::run_failure("assert_failed", "failed"),
        },
        Event::MapItemFinish {
            name: "fan".into(),
            index: 3,
            ms: Milliseconds(5),
        },
        Event::EvalCaseFinish {
            name: "eval".into(),
            index: 0,
        },
        Event::HookStart {
            name: "create".into(),
        },
        Event::HookFinish {
            name: "create".into(),
            status: TaskStatus::Ok,
            ms: Milliseconds(2),
        },
        Event::LogTruncated,
    ];
    for event in &events {
        assert_valid(&schema, "Event", &ser(event, "Event"));
    }

    // RunRecord, both a minimal (required-only) and a full record.
    let minimal = RunRecord {
        id: run_id(),
        flow: None,
        status: RunStatus::Running,
        started_at: Timestamp::new(WHEN),
        finished_at: None,
        ms: None,
        final_state: None,
        results: Vec::new(),
    };
    assert_valid(&schema, "RunRecord", &ser(&minimal, "minimal RunRecord"));
    let full = RunRecord {
        id: run_id(),
        flow: Some("deploy".into()),
        status: RunStatus::Ok,
        started_at: Timestamp::new(WHEN),
        finished_at: Some(Timestamp::new("2026-07-05T12:00:01Z")),
        ms: Some(Milliseconds(1000)),
        final_state: Some(state),
        results: vec![ok_result, err_result],
    };
    assert_valid(&schema, "RunRecord", &ser(&full, "full RunRecord"));
}

#[test]
fn the_validator_rejects_out_of_contract_shapes() {
    let schema = schema_document();

    // An out-of-enum status string is not a RunStatus / TaskStatus / ErrorCategory.
    assert_invalid(&schema, "RunStatus", &json!("exploded"));
    assert_invalid(&schema, "TaskStatus", &json!("pending")); // pending is a RunStatus, not a TaskStatus
    assert_invalid(&schema, "ErrorCategory", &json!("usage")); // usage (exit 2) is not a core category

    // additionalProperties:false: an unexpected key is rejected on a closed record.
    assert_invalid(
        &schema,
        "TaskResult",
        &json!({ "name": "x", "status": "ok", "startedAt": WHEN, "ms": 1, "surprise": true }),
    );
    assert_invalid(
        &schema,
        "MessageWrapper",
        &json!({ "message": "hi", "extra": 1 }),
    );

    // A RunError missing a required field (message) is rejected.
    assert_invalid(
        &schema,
        "RunError",
        &json!({ "category": "validation", "code": "x" }),
    );

    // A RunId that is not a lowercase-hyphenated UUIDv7 fails the pattern.
    assert_invalid(&schema, "RunId", &json!("not-a-uuid"));

    // Milliseconds is non-negative: a negative integer is rejected.
    assert_invalid(&schema, "Milliseconds", &json!(-1));
}
