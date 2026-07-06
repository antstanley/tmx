//! Integration tests for the `PipelineRunner` sequential loop and the `EngineRunFlow` use case,
//! driven over the Task-06 in-memory fake port bundle.
//!
//! These exercise the Task-11 definition of done end to end: a multi-task `assert`/`exec` flow emits
//! the canonical event stream in order and returns the masked final state (O1/O4); the
//! `continueOnError`-vs-abort policy behaves as specified (O1); the load-bearing invariants hold and a
//! too-deep `flow` nest returns `flow_depth_exceeded` (O2). No async runtime is pulled in — the fakes
//! never yield, so a single poll with a no-op waker drives every future to completion, preserving the
//! crate's purity boundary.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::ports::driven::{ChatResponse, ProcessOutput, SourceKind};
use tmx_core::ports::driving::{RunFlow, RunOptions};
use tmx_core::{
    CancelToken, EngineRunFlow, ErrorCategory, Event, Masker, Milliseconds, PipelineRunner,
    PipelineState, Ports, RunConfig, RunError, RunId, RunRecord, RunStatus, TaskStatus,
    resolve_flow,
};
use tmx_testkit::{
    FakeChatModel, FakeHttpClient, FakeReferenceResolver, FakeSchemaValidator, FakeSecretResolver,
    FakeSourceLoader, FixedClock, MemFileSystem, MemObjectStore, RecordingEventSink,
    RecordingProcessRunner, SeededIdGenerator, SerialScheduler,
};

/// Drive an immediately-ready future to completion with a no-op waker — the purity-preserving
/// pattern the rest of the workspace uses, so no async runtime is linked.
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake future must be immediately ready"),
    }
}

/// A bundle of every driven fake the runner needs, owned so a test can seed the builder-form fakes
/// and inspect the recording fakes afterwards.
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

    /// Seed the reference resolver + source loader so `reference` loads `flow_json`.
    fn seed_flow(&mut self, reference: &str, flow_json: Value) {
        let path = format!("{reference}.yaml");
        self.refs =
            FakeReferenceResolver::new().with_reference(reference, path.clone(), SourceKind::Yaml);
        self.loader = FakeSourceLoader::new().with_source(path, flow_json);
    }
}

/// A successful process result capturing `stdout`.
fn stdout(bytes: &[u8]) -> ProcessOutput {
    ProcessOutput {
        exit_code: Some(0),
        stdout: bytes.to_vec(),
        stderr: Vec::new(),
        ms: Milliseconds(0),
    }
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

/// Run `reference` through the `EngineRunFlow` use case over the bundle.
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

#[test]
fn runner_runs_multi_task_flow_emits_ordered_stream_and_masked_state() {
    // O1 + O4: a multi-task `exec` + `assert` flow emits the canonical stream in order and returns
    // the expected final state; the assert reads the prior exec's output through `${{ tasks.* }}`.
    let mut bundle = Bundle::new();
    bundle.process.push_result(Ok(stdout(b"built-ok")));
    bundle.seed_flow(
        "deploy",
        json!({
            "name": "deploy",
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
    );

    let record = run_engine(&bundle, "deploy", json!({})).expect("the run completes");
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the run reaches a terminal ok"
    );
    assert_eq!(record.results.len(), 2, "both tasks produced a result");
    assert_eq!(
        record.results[0].status,
        TaskStatus::Ok,
        "the build task ran and succeeded"
    );

    // The canonical event stream, in order: run.start, then each task's start+finish, then run.finish.
    assert_eq!(
        tags(&bundle.events.events()),
        vec![
            "run.start",
            "task.start",
            "task.finish",
            "task.start",
            "task.finish",
            "run.finish",
        ],
        "the emitted stream matches the canonical order"
    );

    // The returned final state carries both tasks' merged outputs under their names.
    let final_state = record.final_state.expect("a final state was captured");
    assert_eq!(
        final_state.as_value(),
        &json!({
            "build": { "message": "built-ok" },
            "check": { "passed": true, "assertions": 1 }
        }),
        "the merged state matches the expected golden value"
    );
}

#[test]
fn runner_runs_a_chat_completion_task_and_merges_the_completion_into_state() {
    // Task 23 O1/O4: a `chat-completion` task crosses the `ChatModel` port and merges the completion
    // into state under the task's name. Driven over the `FakeChatModel` the completion is deterministic
    // and the request reaching the model is recorded, so both the merged state and the sent prompt are
    // asserted — the same port the `llmRubric` scorer uses (see tests/eval.rs).
    let mut bundle = Bundle::new();
    bundle.chat.push_result(Ok(ChatResponse {
        content: "the-completion-text".to_string(),
        model: "test-model".to_string(),
        prompt_tokens: Some(11),
        completion_tokens: Some(5),
        ms: Milliseconds(0),
    }));
    bundle.seed_flow(
        "ask",
        json!({
            "name": "ask",
            "tasks": [
                {
                    "name": "reply",
                    "type": "chat-completion",
                    "with": {
                        "model": "test-model",
                        "messages": [ { "role": "user", "content": "hello there" } ]
                    }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "ask", json!({})).expect("the chat-completion run completes");
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the chat-completion run reaches a terminal ok"
    );

    // The completion is merged into state under the task name (the dispatcher's `{ content, model }`).
    let final_state = record.final_state.expect("a final state was captured");
    assert_eq!(
        final_state.as_value(),
        &json!({
            "reply": { "content": "the-completion-text", "model": "test-model" }
        }),
        "the completion is merged into state under the task name"
    );

    // The request crossed the port with the resolved model and prompt.
    let requests = bundle.chat.requests();
    assert_eq!(requests.len(), 1, "exactly one completion was requested");
    assert_eq!(
        requests[0].model, "test-model",
        "the model reached the port"
    );
    assert_eq!(
        requests[0].messages.len(),
        1,
        "the single user message reached the port"
    );
}

#[test]
fn runner_masks_a_requested_secret_in_the_output_and_the_final_state() {
    // O1/residue: a task that requests a secret and echoes it cannot surface it — both the emitted
    // task.finish payload and the returned final state are redacted by the run's Masker.
    let mut bundle = Bundle::new();
    bundle.secrets = FakeSecretResolver::new().with_secret("TOKEN_ENV", "supersecretvalue");
    bundle
        .process
        .push_result(Ok(stdout(b"supersecretvalue leaked")));
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

    let record = run_engine(&bundle, "leaky", json!({})).expect("the run completes");
    let final_state = record.final_state.expect("a final state was captured");
    assert_eq!(
        final_state.as_value()["leak"]["message"],
        json!("[REDACTED] leaked"),
        "the secret is redacted out of the final state"
    );

    // The emitted task.finish payload is redacted along the independent event boundary.
    let finish = bundle
        .events
        .events()
        .into_iter()
        .find_map(|event| match event {
            Event::TaskFinish { output, .. } => output,
            _ => None,
        })
        .expect("a task.finish carried output");
    assert_eq!(
        finish["message"], "[REDACTED] leaked",
        "the emitted payload is redacted too"
    );
    let stream = bundle.events.ndjson().expect("the stream serialises");
    assert!(
        !stream.contains("supersecretvalue"),
        "no emitted event leaks the raw secret"
    );
}

#[test]
fn runner_aborts_on_a_failing_non_continue_on_error_task() {
    // O1: a failing assert without continueOnError stops the loop — the following task never starts,
    // and the run is terminal-failed with the failure recorded.
    let mut bundle = Bundle::new();
    bundle.seed_flow(
        "gate",
        json!({
            "name": "gate",
            "tasks": [
                {
                    "name": "gate",
                    "type": "assert",
                    "with": { "assertions": [ { "actual": 1, "matcher": "toBe", "expected": 2 } ] }
                },
                { "name": "after", "type": "exec", "with": { "command": "echo after" } }
            ]
        }),
    );

    let record = run_engine(&bundle, "gate", json!({})).expect("the run completes");
    assert_eq!(
        record.status,
        RunStatus::Failed,
        "the run is terminal-failed"
    );
    assert_eq!(record.results.len(), 1, "the second task never ran");
    assert_eq!(
        record.results[0].status,
        TaskStatus::Error,
        "the gate task is recorded as errored"
    );
    assert_eq!(
        record.results[0].error.as_ref().map(|e| e.code),
        Some("assertion_failed"),
        "the recorded error is the failed assertion"
    );
    assert_eq!(
        tags(&bundle.events.events()),
        vec!["run.start", "task.start", "task.error", "run.finish"],
        "the loop stopped after the failing task"
    );
}

#[test]
fn runner_records_the_error_and_continues_under_continue_on_error() {
    // O1: the same failing assert, now continueOnError, records its error and lets the loop go on to
    // the following task, which runs to completion.
    let mut bundle = Bundle::new();
    bundle.process.push_result(Ok(stdout(b"after")));
    bundle.seed_flow(
        "gate",
        json!({
            "name": "gate",
            "tasks": [
                {
                    "name": "gate",
                    "type": "assert",
                    "continueOnError": true,
                    "with": { "assertions": [ { "actual": 1, "matcher": "toBe", "expected": 2 } ] }
                },
                { "name": "after", "type": "exec", "with": { "command": "echo after" } }
            ]
        }),
    );

    let record = run_engine(&bundle, "gate", json!({})).expect("the run completes");
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "a continueOnError failure does not abort the run"
    );
    assert_eq!(record.results.len(), 2, "both tasks produced a result");
    assert_eq!(
        record.results[0].status,
        TaskStatus::Error,
        "the gate error is still recorded"
    );
    assert_eq!(
        record.results[1].status,
        TaskStatus::Ok,
        "the following task ran"
    );
    assert_eq!(
        tags(&bundle.events.events()),
        vec![
            "run.start",
            "task.start",
            "task.error",
            "task.start",
            "task.finish",
            "run.finish",
        ],
        "the loop continued past the recorded error"
    );
    // The error is recorded in the failing task's state slot, so a downstream task could read it.
    let final_state = record.final_state.expect("a final state was captured");
    assert!(
        final_state.as_value()["gate"]["error"].is_object(),
        "the continueOnError task records its error in state"
    );
}

#[test]
fn runner_skips_a_task_whose_if_gate_is_false() {
    // The `if` gate: a falsy condition emits task.skip, leaves the state unchanged, and does not run
    // the task's effect.
    let mut bundle = Bundle::new();
    bundle.seed_flow(
        "gated",
        json!({
            "name": "gated",
            "inputs": { "enabled": { "default": false } },
            "tasks": [
                { "name": "maybe", "if": "${{ inputs.enabled }}", "type": "exec", "with": { "command": "echo x" } }
            ]
        }),
    );

    let record = run_engine(&bundle, "gated", json!({})).expect("the run completes");
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
        tags(&bundle.events.events()),
        vec!["run.start", "task.skip", "run.finish"],
        "a skipped task emits task.skip, not start/finish"
    );
    assert_eq!(
        bundle.process.calls().len(),
        0,
        "a skipped exec never reaches the process port"
    );
}

#[test]
fn runner_recurses_into_a_sub_flow_within_the_depth_bound() {
    // O1/O2 (positive): a `flow` task within the depth bound loads, runs the sub-flow, and merges its
    // final state as the task's output.
    let mut bundle = Bundle::new();
    bundle.process.push_result(Ok(stdout(b"{\"inner\":true}")));
    // The parent references a sub-flow; both are seeded through the loader.
    bundle.refs = FakeReferenceResolver::new()
        .with_reference("parent", "parent.yaml", SourceKind::Yaml)
        .with_reference("child", "child.yaml", SourceKind::Yaml);
    bundle.loader = FakeSourceLoader::new()
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

    let record = run_engine(&bundle, "parent", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Ok, "the nested run succeeds");
    let final_state = record.final_state.expect("a final state was captured");
    assert_eq!(
        final_state.as_value(),
        &json!({ "sub": { "leaf": { "inner": true } } }),
        "the sub-flow's final state is merged under the flow task's name"
    );
}

#[test]
fn runner_flow_task_past_the_depth_bound_yields_flow_depth_exceeded() {
    // O2 (negative space): a `flow` task at the depth ceiling is rejected with `flow_depth_exceeded`
    // BEFORE any recursion/loading — the guard trips first.
    let bundle = Bundle::new();
    let flow = resolve_flow(json!({
        "name": "nest",
        "tasks": [ { "name": "inner", "type": "flow", "with": { "use": "unseeded" } } ]
    }))
    .expect("the flow resolves");

    let id = RunId::new("018f8c7e-9b2a-7def-8123-456789abcdef").expect("valid id");
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let scheduler = SerialScheduler::new();
    // Start at the depth ceiling: the single flow task's guard sees depth + 1 > FLOW_DEPTH_MAX.
    let depth_ceiling = 8; // FLOW_DEPTH_MAX
    let outcome = block_on_ready(runner.run(
        &id,
        &flow,
        &json!({}),
        bundle.ports(),
        &scheduler,
        &mut masker,
        &mut secrets,
        None,
        depth_ceiling,
    ))
    .expect("the run itself completes (the failure is recorded, not returned)");

    assert_eq!(
        outcome.pipeline.status,
        RunStatus::Failed,
        "a too-deep flow nest fails the run"
    );
    let error = outcome.pipeline.results[0]
        .error
        .as_ref()
        .expect("the flow task recorded an error");
    assert_eq!(
        error.code, "flow_depth_exceeded",
        "the recorded error names the depth bound"
    );
    assert_eq!(
        error.category,
        ErrorCategory::Resolution,
        "a depth overflow is a resolution failure"
    );
    // The guard tripped before any load: the reference resolver was never consulted.
    assert_eq!(
        tags(&bundle.events.events()),
        vec!["run.start", "task.start", "task.error", "run.finish"],
        "the depth guard aborts the single task"
    );
}

#[test]
fn runner_rejects_missing_and_duplicate_task_names() {
    // Negative space for the load-bearing name invariants: a missing or duplicated name is a typed
    // pre-flight error, before the loop.
    let bundle = Bundle::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let id = RunId::new("018f8c7e-9b2a-7def-8123-456789abcdef").expect("valid id");

    let run = |flow_json: Value| -> Result<(), RunError> {
        let flow = resolve_flow(flow_json).expect("the flow resolves");
        let mut masker = Masker::new();
        let mut secrets = Vec::new();
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
        .map(|_| ())
    };

    let missing = run(json!({
        "tasks": [ { "type": "exec", "with": { "command": "echo x" } } ]
    }))
    .expect_err("an unnamed array-form task is rejected");
    assert_eq!(
        missing.code, "missing_task_name",
        "the missing-name pre-flight error names the fault"
    );

    let duplicate = run(json!({
        "tasks": [
            { "name": "dup", "type": "exec", "with": { "command": "a" } },
            { "name": "dup", "type": "exec", "with": { "command": "b" } }
        ]
    }))
    .expect_err("a duplicate name is rejected");
    assert_eq!(
        duplicate.code, "duplicate_task_name",
        "the duplicate-name pre-flight error names the fault"
    );
    assert_eq!(
        duplicate.category,
        ErrorCategory::Validation,
        "a bad name set is a validation failure"
    );
}

#[test]
fn runner_binds_the_matrix_combination_into_every_task_scope() {
    // Task 30 (O1): a `--matrix` combination bound on the runner is visible as `${{ matrix.<key> }}`
    // to the run's tasks — an `assert` that reads both axes holds only because the binding threaded in.
    let bundle = Bundle::new();
    let flow = resolve_flow(json!({
        "name": "m",
        "tasks": [ { "name": "check", "type": "assert", "with": { "assertions": [
            { "actual": "${{ matrix.os }}", "matcher": "toEqual", "expected": "linux" },
            { "actual": "${{ matrix.arch }}", "matcher": "toEqual", "expected": "arm64" }
        ] } } ]
    }))
    .expect("the flow resolves");
    let id = RunId::new("018f8c7e-9b2a-7def-8123-456789abcdef").expect("valid id");

    let run_with = |matrix: Option<Value>| -> RunStatus {
        let runner = match matrix {
            Some(binding) => PipelineRunner::new(RunConfig::default()).with_matrix(binding),
            None => PipelineRunner::new(RunConfig::default()),
        };
        let mut masker = Masker::new();
        let mut secrets = Vec::new();
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
        .expect("the run itself completes")
        .pipeline
        .status
    };

    assert_eq!(
        run_with(Some(json!({ "os": "linux", "arch": "arm64" }))),
        RunStatus::Ok,
        "the assert reading ${{ matrix.os }}/${{ matrix.arch }} holds under the bound combination"
    );
    assert_eq!(
        run_with(Some(json!({ "os": "mac", "arch": "arm64" }))),
        RunStatus::Failed,
        "a different combination fails the same assert — the binding, not a constant, is read"
    );
}

#[test]
fn runner_seeds_prior_state_so_a_sliced_continuation_reads_it() {
    // Task 30 (O1): a `--state-in` seed starts the task loop from prior state, so a later (sliced) task
    // reads an earlier task's output via `${{ tasks.NAME.field }}`; without the seed the read is absent.
    let bundle = Bundle::new();
    let flow = resolve_flow(json!({
        "name": "s",
        "tasks": [ { "name": "check", "type": "assert", "with": { "assertions": [
            { "actual": "${{ tasks.build.sha }}", "matcher": "toEqual", "expected": "abc123" }
        ] } } ]
    }))
    .expect("the flow resolves");
    let seed = PipelineState::new(json!({ "build": { "sha": "abc123" } })).expect("valid state");
    let id = RunId::new("018f8c7e-9b2a-7def-8123-456789abcdef").expect("valid id");

    let run_with = |seed: Option<&PipelineState>| -> (RunStatus, Value) {
        let runner = PipelineRunner::new(RunConfig::default());
        let scheduler = SerialScheduler::new();
        let mut masker = Masker::new();
        let mut secrets = Vec::new();
        let outcome = block_on_ready(runner.run(
            &id,
            &flow,
            &json!({}),
            bundle.ports(),
            &scheduler,
            &mut masker,
            &mut secrets,
            seed,
            0,
        ))
        .expect("the run itself completes");
        (
            outcome.pipeline.status,
            outcome.pipeline.state.as_value().clone(),
        )
    };

    let (seeded_status, seeded_state) = run_with(Some(&seed));
    assert_eq!(
        seeded_status,
        RunStatus::Ok,
        "the resumed task reads the seeded prior state and its assert holds"
    );
    assert_eq!(
        seeded_state.get("build"),
        Some(&json!({ "sha": "abc123" })),
        "the seeded state persists into the run's final state"
    );

    let (unseeded_status, _) = run_with(None);
    assert_eq!(
        unseeded_status,
        RunStatus::Failed,
        "without the seed the prior-state read is absent and the assert fails"
    );
}
