//! Integration tests for lifecycle hooks (Task 12), driven over the Task-06 in-memory fake port
//! bundle.
//!
//! These exercise the Task-12 definition of done end to end: `create`/`change`/`destroy`/`error`
//! fire at exactly their transitions over the fakes (O1); `change` fires once per state-changing task
//! and never on a skip or a no-op merge (O1); `destroy` fires on success, failure, and — through the
//! status-independent finally path — cancellation (O1); a nested-hook attempt trips the one-level
//! assertion and an over-`HOOK_TASKS_MAX` body is rejected (O2); and a full flow with all four hooks
//! shows the ordered `hook.start`/`hook.finish` sequence (O4). No async runtime is linked — the fakes
//! are immediately ready, so a single poll with a no-op waker drives every future to completion,
//! preserving the crate's purity boundary.

use std::future::Future;
use std::pin::pin;
use std::task::{Context as TaskContext, Poll};

use serde_json::{Value, json};

use tmx_core::ports::driven::ProcessOutput;
use tmx_core::ports::driving::{RunFlow, RunOptions};
use tmx_core::{
    EngineRunFlow, Event, HookKind, HookRunner, Masker, Milliseconds, RunConfig, RunError, RunId,
    RunRecord, RunStatus,
};
use tmx_schema::Context;
use tmx_schema::limits::HOOK_TASKS_MAX;
use tmx_testkit::{
    FakeChatModel, FakeHttpClient, FakeReferenceResolver, FakeSchemaValidator, FakeSecretResolver,
    FakeSourceLoader, FixedClock, MemFileSystem, MemObjectStore, RecordingEventSink,
    RecordingProcessRunner, SeededIdGenerator,
};

use tmx_core::Ports;
use tmx_core::ports::driven::SourceKind;

/// Drive an immediately-ready future to completion with a no-op waker.
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = TaskContext::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake future must be immediately ready"),
    }
}

/// A bundle of every driven fake the runner needs.
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
    cancel: tmx_core::CancelToken,
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
            cancel: tmx_core::CancelToken::new(),
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

/// The ordered `(phase, name)` pairs of the hook events in a captured stream — `phase` is `start` or
/// `finish`, `name` is the hook kind.
fn hook_seq(events: &[Event]) -> Vec<(&'static str, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::HookStart { name } => Some(("start", name.clone())),
            Event::HookFinish { name, .. } => Some(("finish", name.clone())),
            _ => None,
        })
        .collect()
}

/// How many times a given hook fired (counts its `hook.start` events).
fn fire_count(events: &[Event], hook: &str) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::HookStart { name } if name == hook))
        .count()
}

/// Run `reference` through the `EngineRunFlow` use case over the bundle.
fn run_engine(bundle: &Bundle, reference: &str, inputs: Value) -> Result<RunRecord, RunError> {
    let use_case = EngineRunFlow::new(bundle.ports(), &bundle.ids, RunConfig::default());
    block_on_ready(use_case.run(reference, inputs, RunOptions::default()))
}

const A_RUN_ID: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

#[test]
fn all_four_hooks_fire_at_their_transitions_in_order() {
    // O1 + O4: a flow declaring all four hooks whose first task changes state (fires `change`) and
    // whose second task aborts the Pipeline (fires `error`). `create` fires on entry, `destroy` fires
    // as the finally — the observed hook sequence is exactly create → change → error → destroy.
    let mut bundle = Bundle::new();
    // Consumed in dispatch order: on_create, task one, on_change, on_error, on_destroy (the aborting
    // assert runs no process).
    for _ in 0..5 {
        bundle.process.push_result(Ok(stdout(b"ok")));
    }
    bundle.seed_flow(
        "lifecycle",
        json!({
            "name": "lifecycle",
            "context": { "hooks": {
                "create":  [ { "name": "on_create",  "type": "exec", "with": { "command": "echo c" } } ],
                "change":  [ { "name": "on_change",  "type": "exec", "with": { "command": "echo h" } } ],
                "error":   [ { "name": "on_error",   "type": "exec", "with": { "command": "echo e" } } ],
                "destroy": [ { "name": "on_destroy", "type": "exec", "with": { "command": "echo d" } } ]
            } },
            "tasks": [
                { "name": "one", "type": "exec", "with": { "command": "echo one" } },
                {
                    "name": "two",
                    "type": "assert",
                    "with": { "assertions": [ { "actual": 1, "matcher": "toBe", "expected": 2 } ] }
                }
            ]
        }),
    );

    let record = run_engine(&bundle, "lifecycle", json!({})).expect("the run completes");
    assert_eq!(
        record.status,
        RunStatus::Failed,
        "the aborting assert fails the run"
    );

    let events = bundle.events.events();
    assert_eq!(
        hook_seq(&events),
        vec![
            ("start", "create".to_string()),
            ("finish", "create".to_string()),
            ("start", "change".to_string()),
            ("finish", "change".to_string()),
            ("start", "error".to_string()),
            ("finish", "error".to_string()),
            ("start", "destroy".to_string()),
            ("finish", "destroy".to_string()),
        ],
        "hooks fire in lifecycle order: create, change (task one), error (task two), destroy",
    );
    // Each hook fired exactly once — a single `change` for the one state-changing task.
    for hook in ["create", "change", "error", "destroy"] {
        assert_eq!(fire_count(&events, hook), 1, "{hook} fires exactly once");
    }
    // Every hook body ran its exec: 5 process calls (create, one, change, error, destroy).
    assert_eq!(
        bundle.process.calls().len(),
        5,
        "each hook body ran through the same runner",
    );
}

#[test]
fn change_fires_once_per_state_changing_task_and_never_on_a_skip() {
    // O1: two runnable tasks each change state (two `change` fires), a third is `if`-gated off and
    // must NOT fire `change` (a skip is not a state change).
    let mut bundle = Bundle::new();
    // Dispatch order: alpha, on_change(alpha), beta, on_change(beta). The skipped task and its hook
    // never run.
    bundle.process.push_result(Ok(stdout(b"a")));
    bundle.process.push_result(Ok(stdout(b"ha")));
    bundle.process.push_result(Ok(stdout(b"b")));
    bundle.process.push_result(Ok(stdout(b"hb")));
    bundle.seed_flow(
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
    );

    let record = run_engine(&bundle, "changes", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Ok, "the run succeeds");
    let events = bundle.events.events();
    assert_eq!(
        fire_count(&events, "change"),
        2,
        "change fires once per state-changing task (alpha, beta) — not for the skipped gamma",
    );
    // The gamma skip is recorded but drove no change hook.
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::TaskSkip { name, .. } if name == "gamma")),
        "the gated task was skipped",
    );
}

#[test]
fn change_does_not_fire_when_the_merge_did_not_change_state() {
    // O1: two tasks writing the same `output` key with an identical value. The first is a state
    // change (fires `change`); the second's merge is a no-op (same value already present), so it must
    // NOT fire `change`.
    let mut bundle = Bundle::new();
    // first (state change) + its change hook, then second (no-op, no hook).
    bundle.process.push_result(Ok(stdout(b"same")));
    bundle.process.push_result(Ok(stdout(b"hook")));
    bundle.process.push_result(Ok(stdout(b"same")));
    bundle.seed_flow(
        "noop",
        json!({
            "name": "noop",
            "context": { "hooks": {
                "change": [ { "name": "on_change", "type": "exec", "with": { "command": "echo x" } } ]
            } },
            "tasks": [
                { "name": "first",  "output": "shared", "type": "exec", "with": { "command": "echo same" } },
                { "name": "second", "output": "shared", "type": "exec", "with": { "command": "echo same" } }
            ]
        }),
    );

    let record = run_engine(&bundle, "noop", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Ok, "the run succeeds");
    assert_eq!(
        fire_count(&bundle.events.events(), "change"),
        1,
        "only the state-changing first task fires change; the no-op second task does not",
    );
}

#[test]
fn destroy_fires_on_both_success_and_failure() {
    // O1: `destroy` is the finally — it fires on a clean run and on an aborted run alike.
    // Success:
    let mut ok = Bundle::new();
    ok.process.push_result(Ok(stdout(b"done"))); // task
    ok.process.push_result(Ok(stdout(b"bye"))); // destroy hook
    ok.seed_flow(
        "clean",
        json!({
            "name": "clean",
            "context": { "hooks": {
                "destroy": [ { "name": "on_destroy", "type": "exec", "with": { "command": "echo d" } } ]
            } },
            "tasks": [ { "name": "work", "type": "exec", "with": { "command": "echo done" } } ]
        }),
    );
    let record = run_engine(&ok, "clean", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Ok, "the clean run succeeds");
    assert_eq!(
        fire_count(&ok.events.events(), "destroy"),
        1,
        "destroy fires on success",
    );

    // Failure:
    let mut bad = Bundle::new();
    bad.process.push_result(Ok(stdout(b"bye"))); // destroy hook only (the assert aborts, runs no exec)
    bad.seed_flow(
        "broken",
        json!({
            "name": "broken",
            "context": { "hooks": {
                "destroy": [ { "name": "on_destroy", "type": "exec", "with": { "command": "echo d" } } ]
            } },
            "tasks": [ {
                "name": "gate",
                "type": "assert",
                "with": { "assertions": [ { "actual": 1, "matcher": "toBe", "expected": 2 } ] }
            } ]
        }),
    );
    let record = run_engine(&bad, "broken", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Failed, "the broken run fails");
    assert_eq!(
        fire_count(&bad.events.events(), "destroy"),
        1,
        "destroy still fires on failure — the finally",
    );
}

#[test]
fn destroy_fires_through_the_status_independent_finally_path() {
    // O1 (cancellation): the runner cannot yet reach a cancelled terminal status (Task 29), but
    // `destroy` firing is status-independent — the finally invokes exactly this `HookRunner::fire`
    // path regardless of the terminal status. Driving it directly is representative of the cancelled
    // path, not a stub of it.
    let bundle = Bundle::new();
    bundle.process.push_result(Ok(stdout(b"bye")));
    let ctx: Context = serde_json::from_value(json!({
        "hooks": { "destroy": [ { "name": "on_destroy", "type": "exec", "with": { "command": "echo d" } } ] }
    }))
    .expect("the context parses");
    let hooks = HookRunner::new(Some(&ctx), RunConfig::default(), false);
    let id = RunId::new(A_RUN_ID).expect("valid id");
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let fired = block_on_ready(hooks.fire(
        HookKind::Destroy,
        &id,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        0,
    ))
    .expect("the destroy hook runs");
    assert!(fired, "the destroy hook body fired");
    assert_eq!(
        hook_seq(&bundle.events.events()),
        vec![
            ("start", "destroy".to_string()),
            ("finish", "destroy".to_string()),
        ],
        "the finally emits a single destroy hook.start/hook.finish pair",
    );
}

#[test]
fn a_change_hook_that_mutates_state_does_not_re_trigger_change() {
    // O2 (negative space, no hook-storm): the `change` hook body itself runs a state-changing task,
    // but a hook body runs one level deep and fires no hooks — so `change` fires exactly once, not in
    // a storm.
    let mut bundle = Bundle::new();
    bundle.process.push_result(Ok(stdout(b"task"))); // the state-changing task
    bundle.process.push_result(Ok(stdout(b"mutate"))); // the change hook's own mutating task
    bundle.seed_flow(
        "storm",
        json!({
            "name": "storm",
            "context": { "hooks": {
                "change": [ { "name": "mutate", "type": "exec", "with": { "command": "echo mutate" } } ]
            } },
            "tasks": [ { "name": "one", "type": "exec", "with": { "command": "echo task" } } ]
        }),
    );

    let record = run_engine(&bundle, "storm", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Ok, "the run succeeds");
    assert_eq!(
        fire_count(&bundle.events.events(), "change"),
        1,
        "the change hook body's own state change does not re-trigger change (no hook-storm)",
    );
}

#[test]
#[should_panic(expected = "one level deep")]
fn firing_a_hook_while_already_inside_one_trips_the_assertion() {
    // O2: the asserted backstop. A HookRunner marked as already inside a hook must refuse to fire a
    // lifecycle hook — the one-level-deep invariant, asserted.
    let bundle = Bundle::new();
    let ctx: Context = serde_json::from_value(json!({
        "hooks": { "change": [ { "name": "x", "type": "exec", "with": { "command": "echo x" } } ] }
    }))
    .expect("the context parses");
    // `in_hook = true`: this runner stands for one already executing a hook body.
    let hooks = HookRunner::new(Some(&ctx), RunConfig::default(), true);
    let id = RunId::new(A_RUN_ID).expect("valid id");
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let _ = block_on_ready(hooks.fire(
        HookKind::Change,
        &id,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        0,
    ));
}

#[test]
fn an_over_limit_hook_body_is_rejected() {
    // O2 (negative space): a hook body with more than HOOK_TASKS_MAX tasks is a typed error, rejected
    // before it runs.
    let bundle = Bundle::new();
    let over = (HOOK_TASKS_MAX as usize) + 1;
    let tasks: Vec<Value> = (0..over)
        .map(
            |i| json!({ "name": format!("t{i}"), "type": "exec", "with": { "command": "echo x" } }),
        )
        .collect();
    let ctx: Context = serde_json::from_value(json!({ "hooks": { "create": tasks } }))
        .expect("the context parses");
    let hooks = HookRunner::new(Some(&ctx), RunConfig::default(), false);
    let id = RunId::new(A_RUN_ID).expect("valid id");
    let mut masker = Masker::new();
    let mut secrets = Vec::new();
    let err = block_on_ready(hooks.fire(
        HookKind::Create,
        &id,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        0,
    ))
    .expect_err("an over-limit hook body is rejected");
    assert_eq!(
        err.code, "too_many_hook_tasks",
        "the rejection names the hook-task bound",
    );
    // Rejected before any hook event was emitted.
    assert!(
        bundle.events.events().is_empty(),
        "the over-limit body never emitted a hook.start",
    );
}

#[test]
fn a_hook_free_flow_emits_no_hook_events() {
    // Regression (P3): a flow with no declared hooks runs exactly as before — no hook firing points
    // leak into the stream.
    let mut bundle = Bundle::new();
    bundle.process.push_result(Ok(stdout(b"done")));
    bundle.seed_flow(
        "plain",
        json!({
            "name": "plain",
            "tasks": [ { "name": "work", "type": "exec", "with": { "command": "echo done" } } ]
        }),
    );
    let record = run_engine(&bundle, "plain", json!({})).expect("the run completes");
    assert_eq!(record.status, RunStatus::Ok, "the plain run succeeds");
    assert!(
        hook_seq(&bundle.events.events()).is_empty(),
        "a hook-free flow emits no hook events",
    );
}
