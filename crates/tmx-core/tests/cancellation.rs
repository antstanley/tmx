//! Integration tests for cancellation, timeout, and interrupt (Task 29), driven over the Task-06
//! in-memory fakes plus a deliberately *hanging* process runner.
//!
//! These exercise the Task-29 definition of done end to end at the core seam: a run whose in-flight
//! adapter ignores the grace period is **hard-stopped** when hard cancellation fires (O2); the run
//! ends `timed_out`/`cancelled` (not `failed`), the `destroy` hook still fires — the best-effort
//! finally — and no work is dispatched past the cancellation point (O1). A soft-cancel requested
//! *between* tasks stops dispatch cleanly without cutting a task off mid-flight (O1). The
//! `--timeout`→124 / SIGINT→130 mapping itself is unit-tested at the `main` seam (`tmx-cli`); here we
//! prove the engine reaches the terminal `timed_out`/`cancelled` status those codes map from.
//!
//! No async runtime is linked: a fake future is immediately ready, and the one hanging future is
//! driven by a hand-rolled poller that triggers cancellation between polls — the same
//! purity-preserving discipline the rest of the core tests use, with the hard stop injected by hand.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::cancel::CancelReason;
use tmx_core::ports::driven::{ProcessOutput, ProcessRunner, ProcessSpec};
use tmx_core::{
    CancelToken, Event, Masker, PipelineRunner, Ports, RunConfig, RunError, RunId, RunStatus,
    resolve_flow,
};
use tmx_testkit::{
    FakeChatModel, FakeHttpClient, FakeReferenceResolver, FakeSchemaValidator, FakeSecretResolver,
    FakeSourceLoader, FixedClock, MemFileSystem, MemObjectStore, RecordingEventSink,
    RecordingProcessRunner,
};

/// A [`ProcessRunner`] whose `run` never returns — the adapter that ignores the grace period, so the
/// only way the run stops is the hard cancellation dropping this future (O2).
#[derive(Debug, Default)]
struct HangingProcessRunner;

#[async_trait::async_trait]
impl ProcessRunner for HangingProcessRunner {
    async fn run(&self, _spec: ProcessSpec) -> Result<ProcessOutput, RunError> {
        // Never resolves: an adapter that does not observe cancellation itself. The runner's
        // cancellation guard must drop this future at the hard-stop deadline.
        std::future::pending().await
    }
}

/// Drive an immediately-ready future with a no-op waker (the fakes never yield).
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a ready future must complete on the first poll"),
    }
}

/// Drive `fut` to completion, invoking `on_first_pending` exactly once the first time it parks — the
/// seam where a test injects a hard cancellation while the hanging adapter is in flight. After the
/// trigger every remaining await is on a ready fake, so the future resolves promptly; a bounded poll
/// budget turns a hypothetical wedge into a test failure rather than a hang.
fn drive_with_trigger<F: Future>(fut: F, on_first_pending: impl FnOnce()) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    let mut trigger = Some(on_first_pending);
    for _ in 0..1024 {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => {
                if let Some(trigger) = trigger.take() {
                    trigger();
                }
            }
        }
    }
    panic!("the run did not resolve after cancellation was triggered — a cancelled run is hostage");
}

/// The owned fake bundle for a cancellation run, generic over the process runner so a test can inject
/// either the hanging runner or the ordinary recording one.
struct Bundle<P: ProcessRunner> {
    process: P,
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
    cancel: CancelToken,
}

impl<P: ProcessRunner> Bundle<P> {
    fn with_process(process: P) -> Self {
        Self {
            process,
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
}

/// A run id every test shares — the runner never validates externally-minted ids past construction.
fn run_id() -> RunId {
    match RunId::new("018f8c7e-9b2a-7def-8123-456789abcdef") {
        Ok(id) => id,
        Err(_) => unreachable!("the literal is a well-formed UUIDv7"),
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

/// The names of every `task.start` in the stream — including hook-body inner tasks, which emit their
/// own `task.start`/`task.finish`, so a test must filter by name to reason about *flow* tasks alone.
fn task_start_names(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::TaskStart { name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// A flow whose first task hangs, a second task follows (must never dispatch), and a `destroy` hook
/// runs a pure `assert` (so the finally completes without touching the hanging process runner).
fn hanging_flow() -> Value {
    json!({
        "name": "hang",
        "context": { "hooks": {
            "destroy": [ {
                "name": "on_destroy",
                "type": "assert",
                "with": { "assertions": [ { "actual": true, "matcher": "toBe", "expected": true } ] }
            } ]
        } },
        "tasks": [
            { "name": "slow", "type": "exec", "with": { "command": "sleep 999" } },
            { "name": "after", "type": "exec", "with": { "command": "echo after" } }
        ]
    })
}

#[test]
fn a_hard_cancelled_in_flight_task_ends_timed_out_fires_destroy_and_stops_dispatch() {
    // O1 + O2: the first task hangs (an adapter ignoring the grace period). A hard `--timeout`
    // cancellation fires while it is in flight; the guard drops it (hard stop), the run ends
    // `timed_out` (→ exit 124), the `destroy` finally still fires, and the second task is never
    // dispatched (no work past the cancellation point).
    let bundle = Bundle::with_process(HangingProcessRunner);
    let flow = resolve_flow(hanging_flow()).expect("the flow resolves");
    let id = run_id();
    let mut masker = Masker::new();
    let mut secrets: Vec<String> = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());

    let cancel = bundle.cancel.clone();
    let outcome = drive_with_trigger(
        runner.run(
            &id,
            &flow,
            &json!({}),
            bundle.ports(),
            &mut masker,
            &mut secrets,
            0,
        ),
        // While the hanging task is parked, escalate straight to a hard timeout cancellation (the
        // grace window is the CLI's concern; here we exercise the hard stop the runner reacts to).
        || cancel.hard_cancel(CancelReason::Timeout),
    )
    .expect("a cancelled run returns a terminal outcome, not an Err");

    assert_eq!(
        outcome.pipeline.status,
        RunStatus::TimedOut,
        "a --timeout hard cancel ends the run `timed_out` (→ exit 124), not `failed`"
    );

    let events = bundle.events.events();
    let stream = tags(&events);
    // The `destroy` finally fired even though the run was cancelled mid-flight.
    assert!(
        stream.iter().any(|t| t == "hook.start") && stream.iter().any(|t| t == "hook.finish"),
        "the destroy hook fired as the finally after cancellation, got {stream:?}"
    );
    // The first flow task started and was cut off; the second flow task never dispatched (the
    // `on_destroy` hook task's own start is a hook-body task, filtered out by name here).
    let started = task_start_names(&events);
    assert!(
        started.contains(&"slow".to_string()),
        "the cut-off first task started, got {started:?}"
    );
    assert!(
        !started.contains(&"after".to_string()),
        "the second flow task never dispatched past the cancellation point, got {started:?}"
    );
    // Only the cut-off task is recorded, as an error — the run holds no result for the undispatched one.
    assert_eq!(
        outcome.pipeline.results.len(),
        1,
        "only the in-flight task is recorded"
    );
    assert_eq!(outcome.pipeline.results[0].name, "slow", "the cut-off task");
    assert_eq!(
        outcome.pipeline.results[0].status,
        tmx_core::TaskStatus::Error,
        "the cut-off task is recorded as errored"
    );
}

#[test]
fn a_soft_cancel_requested_before_a_task_stops_dispatch_cleanly() {
    // O1 negative-space companion: a cancellation *requested* before the loop reaches a task stops
    // dispatch at the top of the loop — cleanly, with no task started and no task cut off. The run
    // still ends terminal (`cancelled` → exit 130) and the `destroy` finally still fires.
    let bundle = Bundle::with_process(RecordingProcessRunner::new());
    let flow = resolve_flow(hanging_flow()).expect("the flow resolves");
    // Request an interrupt up front: the very first loop iteration sees it and breaks before any
    // dispatch. (Only requested — never hard — so this is the clean stop-dispatch path.)
    bundle.cancel.request(CancelReason::Interrupt);

    let id = run_id();
    let mut masker = Masker::new();
    let mut secrets: Vec<String> = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let outcome = block_on_ready(runner.run(
        &id,
        &flow,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        0,
    ))
    .expect("a cancelled run returns a terminal outcome");

    assert_eq!(
        outcome.pipeline.status,
        RunStatus::Cancelled,
        "a requested SIGINT ends the run `cancelled` (→ exit 130)"
    );
    assert!(
        outcome.pipeline.results.is_empty(),
        "no task was dispatched — the stop happened before the first task"
    );
    let events = bundle.events.events();
    let stream = tags(&events);
    // No *flow* task started (the `on_destroy` hook-body task's start is filtered out by name).
    let started = task_start_names(&events);
    assert!(
        !started.contains(&"slow".to_string()) && !started.contains(&"after".to_string()),
        "no flow task started before the requested cancellation, got {started:?}"
    );
    assert!(
        stream.iter().any(|t| t == "hook.start"),
        "the destroy finally still fired on the cancelled run, got {stream:?}"
    );
    // The recording process runner was never asked to run anything.
    assert!(
        bundle.process.calls().is_empty(),
        "no process was dispatched on a pre-requested cancellation"
    );
}

#[test]
fn a_never_triggered_token_runs_the_flow_to_completion_unaffected() {
    // Regression (the certificate's regression check): the always-present token is a no-op when never
    // triggered — a fast flow with no cancellation completes through the guarded step exactly as
    // before, reaching `ok` with both tasks run.
    let bundle = Bundle::with_process(
        RecordingProcessRunner::new()
            .with_stdout(b"one".to_vec())
            .with_stdout(b"two".to_vec()),
    );
    let flow = resolve_flow(json!({
        "name": "clean",
        "tasks": [
            { "name": "a", "type": "exec", "with": { "command": "echo one" } },
            { "name": "b", "type": "exec", "with": { "command": "echo two" } }
        ]
    }))
    .expect("the flow resolves");

    let id = run_id();
    let mut masker = Masker::new();
    let mut secrets: Vec<String> = Vec::new();
    let runner = PipelineRunner::new(RunConfig::default());
    let outcome = block_on_ready(runner.run(
        &id,
        &flow,
        &json!({}),
        bundle.ports(),
        &mut masker,
        &mut secrets,
        0,
    ))
    .expect("the run completes");

    assert_eq!(
        outcome.pipeline.status,
        RunStatus::Ok,
        "a never-triggered token leaves the run to reach `ok`"
    );
    assert_eq!(
        outcome.pipeline.results.len(),
        2,
        "both tasks ran under the un-triggered token"
    );
    assert_eq!(
        bundle.process.calls().len(),
        2,
        "both process tasks were dispatched — the guard added no interference"
    );
}
