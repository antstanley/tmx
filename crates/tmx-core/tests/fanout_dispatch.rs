//! End-to-end `map`/`eval` dispatch through the runner (Task 33).
//!
//! Tasks 18/19 built `run_map`/`run_eval` as standalone engine units, but no task wired the two
//! control-flow task types into the runner's per-task dispatch — a Flow containing a `map` or `eval`
//! task returned `task_type_unsupported` at runtime. These tests drive a `map` Flow and an `eval` Flow
//! all the way through the [`EngineRunFlow`] (`RunFlow`) use case over the deterministic
//! `tmx-testkit` fakes, asserting the fan-out executes, collects in item order, emits its per-element
//! events, produces a `Scorecard`, and gates on a `threshold` — and that every `run_map`/`run_eval`
//! guard still surfaces through the runner. No async runtime is linked: the fakes never yield, so a
//! single poll with a no-op waker drives every future to completion.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::ports::driven::{ProcessOutput, SourceKind};
use tmx_core::ports::driving::{RunFlow, RunOptions};
use tmx_core::{
    CancelToken, EngineRunFlow, Event, Milliseconds, Ports, RunConfig, RunError, RunRecord,
    RunStatus,
};
use tmx_testkit::{
    FakeChatModel, FakeHttpClient, FakeReferenceResolver, FakeSchemaValidator, FakeSecretResolver,
    FakeSourceLoader, FixedClock, MemFileSystem, MemObjectStore, RecordingEventSink,
    RecordingProcessRunner, SeededIdGenerator, SerialScheduler,
};

/// Drive an immediately-ready future to completion with a no-op waker — the workspace's
/// purity-preserving pattern (the fakes and the serial scheduler complete on the first poll).
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake-backed run must be immediately ready"),
    }
}

/// The full deterministic fake port bundle plus the always-serial scheduler.
struct Bundle {
    process: RecordingProcessRunner,
    http: FakeHttpClient,
    fs: MemFileSystem,
    store: MemObjectStore,
    chat: FakeChatModel,
    clock: FixedClock,
    events: RecordingEventSink,
    secrets: FakeSecretResolver,
    schema: FakeSchemaValidator,
    refs: FakeReferenceResolver,
    loader: FakeSourceLoader,
    ids: SeededIdGenerator,
    cancel: CancelToken,
}

impl Bundle {
    fn new() -> Self {
        Self {
            process: RecordingProcessRunner::new(),
            http: FakeHttpClient::new(),
            fs: MemFileSystem::new(),
            store: MemObjectStore::new(),
            chat: FakeChatModel::new(),
            clock: FixedClock::new(),
            events: RecordingEventSink::new(),
            secrets: FakeSecretResolver::new(),
            schema: FakeSchemaValidator::new(),
            refs: FakeReferenceResolver::new(),
            loader: FakeSourceLoader::new(),
            ids: SeededIdGenerator::new(),
            cancel: CancelToken::new(),
        }
    }

    fn ports(&self) -> Ports<'_> {
        Ports {
            process: &self.process,
            http: &self.http,
            file: &self.fs,
            store: &self.store,
            chat: &self.chat,
            clock: &self.clock,
            events: &self.events,
            secrets: &self.secrets,
            schema: &self.schema,
            reference_resolver: &self.refs,
            source_loader: &self.loader,
            cancel: &self.cancel,
        }
    }

    fn seed_flow(&mut self, reference: &str, flow_json: Value) {
        let path = format!("{reference}.yaml");
        self.refs =
            FakeReferenceResolver::new().with_reference(reference, path.clone(), SourceKind::Yaml);
        self.loader = FakeSourceLoader::new().with_source(path, flow_json);
    }
}

/// A successful process result capturing `stdout` (exit 0).
fn stdout(bytes: &[u8]) -> ProcessOutput {
    ProcessOutput {
        exit_code: Some(0),
        stdout: bytes.to_vec(),
        stderr: Vec::new(),
        ms: Milliseconds(0),
    }
}

/// Run `reference` through the `EngineRunFlow` use case over the bundle and the serial scheduler.
fn run_engine(bundle: &Bundle, reference: &str, inputs: Value) -> Result<RunRecord, RunError> {
    let scheduler = SerialScheduler::new();
    let use_case = EngineRunFlow::new(
        bundle.ports(),
        &bundle.ids,
        &scheduler,
        RunConfig::default(),
    );
    block_on_ready(use_case.run(reference, inputs, RunOptions::default()))
}

/// The `event` tags of a captured stream, in order.
fn tags(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            serde_json::to_value(event).unwrap_or_else(|e| panic!("event serialises: {e}"))["event"]
                .as_str()
                .unwrap_or_else(|| panic!("event tag is a string"))
                .to_string()
        })
        .collect()
}

#[test]
fn a_map_flow_runs_end_to_end_and_collects_the_output_array_in_item_order() {
    // O1: a Flow with a `map` task fans out over its `items`, running the inner task once per element,
    // and collects the per-item outputs into an array **in item order** — no `task_type_unsupported`.
    let mut bundle = Bundle::new();
    // The inner exec's output is scripted per element (consumed in index order by the serial
    // scheduler); the collected array must follow that order.
    bundle.process.push_result(Ok(stdout(b"10")));
    bundle.process.push_result(Ok(stdout(b"20")));
    bundle.process.push_result(Ok(stdout(b"30")));
    bundle.seed_flow(
        "fanflow",
        json!({
            "name": "fanflow",
            "tasks": [
                {
                    "name": "fan",
                    "type": "map",
                    "with": {
                        "items": ["a", "b", "c"],
                        "task": { "type": "exec", "with": { "command": "echo ${{ item }}" } }
                    }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "fanflow", json!({})).expect("the map flow runs");
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "a map flow reaches a terminal ok — the runner no longer rejects map/eval"
    );
    let state = record.final_state.expect("a final state was captured");
    assert_eq!(
        state.as_value().get("fan"),
        Some(&json!([10, 20, 30])),
        "the map collects one output per item, in item order"
    );

    // The per-element `map.item.finish` events fire (one per item), between the map task's own
    // `task.start` and `task.finish`.
    let stream = tags(&bundle.events.events());
    let item_finishes = stream.iter().filter(|t| *t == "map.item.finish").count();
    assert_eq!(item_finishes, 3, "one map.item.finish per element");
    assert!(
        !stream.iter().any(|t| t == "task.error"),
        "a wired map emits no task.error (it was never task_type_unsupported)"
    );
}

#[test]
fn a_map_binds_each_element_under_item_so_the_inner_task_reads_it() {
    // O1 (binding): the inner task reads the bound element via `${{ item.* }}`; an inner `assert` that
    // gates on `${{ item.n }}` passes for every element, proving the per-element binding threaded in.
    let mut bundle = Bundle::new();
    bundle.seed_flow(
        "boundflow",
        json!({
            "name": "boundflow",
            "tasks": [
                {
                    "name": "fan",
                    "type": "map",
                    "with": {
                        "items": [{ "n": 1 }, { "n": 2 }],
                        "task": {
                            "type": "assert",
                            "with": { "assertions": [
                                { "actual": "${{ item.n }}", "matcher": "toBeGreaterThan", "expected": 0 }
                            ] }
                        }
                    }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "boundflow", json!({})).expect("the bound map runs");
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "every element's `${{ item.n }}` bound and passed its gate"
    );
    let state = record.final_state.expect("a final state was captured");
    let collected = state
        .as_value()
        .get("fan")
        .and_then(Value::as_array)
        .expect("the map output is an array");
    assert_eq!(collected.len(), 2, "one output slot per element");
    assert_eq!(
        collected[0],
        json!({ "passed": true, "assertions": 1 }),
        "the first element's inner assert passed"
    );
}

#[test]
fn an_eval_flow_runs_end_to_end_and_emits_a_scorecard() {
    // O2: a Flow with an `eval` task runs the subject once per case, scores it, and merges a
    // `Scorecard` under the task name; the per-case `eval.case.finish` events fire.
    let mut bundle = Bundle::new();
    // The subject echoes each case's `expected` value as the output to score (two cases).
    bundle.process.push_result(Ok(stdout(b"\"hi\"")));
    bundle.process.push_result(Ok(stdout(b"\"hi\"")));
    bundle.seed_flow(
        "evalflow",
        json!({
            "name": "evalflow",
            "tasks": [
                {
                    "name": "quality",
                    "type": "eval",
                    "with": {
                        "dataset": [{ "expected": "hi" }, { "expected": "hi" }],
                        "subject": { "type": "exec", "with": { "command": "echo hi" } },
                        "scorers": [
                            { "name": "exact", "type": "matcher", "matcher": "toEqual", "expected": "${{ case.expected }}" }
                        ],
                        "threshold": { "metric": "weightedMean", "min": 0.5 }
                    }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "evalflow", json!({})).expect("the eval flow runs");
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the eval meets its threshold and the run is ok"
    );
    let state = record.final_state.expect("a final state was captured");
    let scorecard = state
        .as_value()
        .get("quality")
        .expect("the eval merged a scorecard under its name");
    assert_eq!(
        scorecard
            .get("cases")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "the scorecard carries one case per dataset entry"
    );
    assert_eq!(
        scorecard.get("passed"),
        Some(&Value::Bool(true)),
        "the scorecard reports the run passed its threshold"
    );
    assert!(
        scorecard.get("summary").is_some(),
        "the scorecard carries an aggregate summary"
    );

    let stream = tags(&bundle.events.events());
    let case_finishes = stream.iter().filter(|t| *t == "eval.case.finish").count();
    assert_eq!(case_finishes, 2, "one eval.case.finish per scored case");
}

#[test]
fn an_eval_that_misses_its_threshold_fails_the_run() {
    // O2 (negative space): a `threshold` the eval does not meet is a typed `eval_threshold_missed`
    // RunFailure surfaced through the runner — the run ends `failed`.
    let mut bundle = Bundle::new();
    // Case 0 matches (score 1.0); case 1 does not (score 0.0) → weightedMean 0.5, below min 0.9.
    bundle.process.push_result(Ok(stdout(b"\"hi\"")));
    bundle.process.push_result(Ok(stdout(b"\"hi\"")));
    bundle.seed_flow(
        "gateflow",
        json!({
            "name": "gateflow",
            "tasks": [
                {
                    "name": "quality",
                    "type": "eval",
                    "with": {
                        "dataset": [{ "expected": "hi" }, { "expected": "bye" }],
                        "subject": { "type": "exec", "with": { "command": "echo hi" } },
                        "scorers": [
                            { "name": "exact", "type": "matcher", "matcher": "toEqual", "expected": "${{ case.expected }}" }
                        ],
                        "threshold": { "metric": "weightedMean", "min": 0.9 }
                    }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "gateflow", json!({})).expect("the run itself completes");
    assert_eq!(
        record.status,
        RunStatus::Failed,
        "a missed threshold fails the run"
    );
    let error = record.results[0]
        .error
        .as_ref()
        .expect("the eval task recorded a failure");
    assert_eq!(
        error.code, "eval_threshold_missed",
        "the recorded error names the missed threshold"
    );
}

#[test]
fn a_map_whose_items_do_not_resolve_to_an_array_surfaces_the_typed_guard() {
    // O3: the `run_map` guards still surface through the runner — an `items` expression resolving to a
    // non-array is a typed `map_items_not_array`, recorded as the task's failure.
    let mut bundle = Bundle::new();
    bundle.seed_flow(
        "badflow",
        json!({
            "name": "badflow",
            "inputs": { "count": { "type": "number" } },
            "tasks": [
                {
                    "name": "fan",
                    "type": "map",
                    "with": {
                        "items": "${{ inputs.count }}",
                        "task": { "type": "exec", "with": { "command": "echo x" } }
                    }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "badflow", json!({ "count": 5 })).expect("the run completes");
    assert_eq!(
        record.status,
        RunStatus::Failed,
        "a non-array items fails the run, not a silent empty fan-out"
    );
    let error = record.results[0]
        .error
        .as_ref()
        .expect("the map task recorded a failure");
    assert_eq!(
        error.code, "map_items_not_array",
        "the runtime guard surfaces unchanged through the runner"
    );
}
