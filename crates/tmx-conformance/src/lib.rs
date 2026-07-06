#![forbid(unsafe_code)]
//! `tmx-conformance` — the workspace-level golden-Flow conformance harness.
//!
//! This crate is the shared spine of the [conformance tier](../../.specs/plans/2026-07-05-tmx_runtime_implementation/done/32-conformance_suite.md):
//! a single injectable bundle of the deterministic `tmx-testkit` fakes plus the helpers that drive
//! [`RunFlow`](tmx_core::ports::driving::RunFlow) over them. The actual cases — golden Flows, the
//! limit-boundary trio, the negative-space cases, and the property-test tier — live under `tests/`
//! and share this harness so every case is driven the *same* deterministic way.
//!
//! ## Determinism
//!
//! Every run is seeded only by the three determinism fakes — [`FixedClock`], [`SeededIdGenerator`],
//! and (for fan-out) `SerialScheduler` — so two fresh [`Bundle`]s driving the same Flow emit
//! byte-identical event streams and final state ([architecture-principles.md](../../.specs/architecture-principles.md)
//! §2.5). No `SystemTime::now()`, no randomness, and no async runtime: the fakes are immediately
//! ready, so [`block_on_ready`] drives every future to completion on a single poll with a no-op
//! waker, exactly as the rest of the workspace does — the crate stays inside the purity boundary.
//!
//! The harness itself takes no `.unwrap()`/`.expect()` (those are denied outside test bodies); it
//! surfaces failures through explicit `panic!`, and the `#[test]` bodies under `tests/` use the
//! usual test-only `unwrap`/`expect`.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::Value;

use tmx_core::ports::driven::{ProcessOutput, SourceKind};
use tmx_core::ports::driving::{RunFlow, RunOptions};
use tmx_core::{
    CancelToken, EngineRunFlow, Event, Milliseconds, Ports, RunConfig, RunError, RunRecord, Scope,
};
use tmx_testkit::{
    FakeChatModel, FakeHttpClient, FakeReferenceResolver, FakeSchemaValidator, FakeSecretResolver,
    FakeSourceLoader, FixedClock, MemFileSystem, MemObjectStore, RecordingEventSink,
    RecordingProcessRunner, SeededIdGenerator,
};

/// Drive an immediately-ready future to completion with a no-op waker — the workspace's
/// purity-preserving pattern, so no async runtime is linked. The testkit fakes never yield, so the
/// first poll is always [`Poll::Ready`]; a `Pending` would be a harness bug, so it `panic!`s.
pub fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake-backed conformance future must be immediately ready"),
    }
}

/// A bundle of every driven fake the engine needs, owned so a case can script the builder-form
/// fakes up front and inspect the recording fakes afterwards.
///
/// Fields are public so a case both seeds a port (e.g. `bundle.process.push_result(..)`) and reads
/// its recording (e.g. `bundle.events.events()`). [`Bundle::new`] uses the default seeds, so two
/// fresh bundles are byte-identical until one is scripted.
pub struct Bundle {
    /// The scripted, recording process runner (`exec`/`run`).
    pub process: RecordingProcessRunner,
    /// The canned-response, recording HTTP client (`fetch`).
    pub http: FakeHttpClient,
    /// The in-memory filesystem (`file`).
    pub fs: MemFileSystem,
    /// The in-memory object store (`store`).
    pub store: MemObjectStore,
    /// The canned-completion, recording chat model (`chat-completion`).
    pub chat: FakeChatModel,
    /// The frozen, step-advanceable clock (a determinism seam).
    pub clock: FixedClock,
    /// The capturing event sink (the asserted event stream).
    pub events: RecordingEventSink,
    /// The seeded secret resolver.
    pub secrets: FakeSecretResolver,
    /// The scripted schema validator.
    pub schema: FakeSchemaValidator,
    /// The seeded reference resolver.
    pub refs: FakeReferenceResolver,
    /// The seeded source loader.
    pub loader: FakeSourceLoader,
    /// The seeded UUIDv7 id generator (a determinism seam).
    pub ids: SeededIdGenerator,
    /// The run's cancellation token (never triggered in the golden tier).
    pub cancel: CancelToken,
}

impl Bundle {
    /// Assemble the full deterministic fake port set with default seeds.
    #[must_use]
    pub fn new() -> Self {
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

    /// Borrow every fake as the runner's [`Ports`] bundle.
    #[must_use]
    pub fn ports(&self) -> Ports<'_> {
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

    /// Seed the reference resolver + source loader so `reference` loads `flow_json` (as YAML).
    pub fn seed_flow(&mut self, reference: &str, flow_json: Value) {
        let path = format!("{reference}.yaml");
        self.refs =
            FakeReferenceResolver::new().with_reference(reference, path.clone(), SourceKind::Yaml);
        self.loader = FakeSourceLoader::new().with_source(path, flow_json);
    }
}

impl Default for Bundle {
    fn default() -> Self {
        Self::new()
    }
}

/// A successful process result capturing `stdout` (exit 0, no stderr, zero duration).
#[must_use]
pub fn stdout(bytes: &[u8]) -> ProcessOutput {
    ProcessOutput {
        exit_code: Some(0),
        stdout: bytes.to_vec(),
        stderr: Vec::new(),
        ms: Milliseconds(0),
    }
}

/// Run `reference` through the [`EngineRunFlow`] use case over the bundle with the default engine
/// config.
///
/// # Errors
///
/// Propagates the use case's typed [`RunError`] (a pre-flight abort or an output-port failure);
/// a task-level failure is a terminal `RunRecord`, not an `Err`.
pub fn run_engine(bundle: &Bundle, reference: &str, inputs: Value) -> Result<RunRecord, RunError> {
    run_engine_with_config(bundle, reference, inputs, RunConfig::default())
}

/// Run `reference` through the [`EngineRunFlow`] use case over the bundle with an explicit
/// [`RunConfig`] (e.g. a narrowed `max_state_size_bytes` for the over-cap boundary).
///
/// # Errors
///
/// Propagates the use case's typed [`RunError`].
pub fn run_engine_with_config(
    bundle: &Bundle,
    reference: &str,
    inputs: Value,
    config: RunConfig,
) -> Result<RunRecord, RunError> {
    let use_case = EngineRunFlow::new(bundle.ports(), &bundle.ids, config);
    block_on_ready(use_case.run(reference, inputs, RunOptions::default()))
}

/// The `event` tag of one event (its serialised discriminator, e.g. `"task.start"`). A serialisation
/// failure is a harness bug, so it `panic!`s rather than swallowing it.
#[must_use]
pub fn event_tag(event: &Event) -> String {
    let value = serde_json::to_value(event)
        .unwrap_or_else(|error| panic!("an event must serialise: {error}"));
    value["event"]
        .as_str()
        .unwrap_or_else(|| panic!("an event's `event` discriminator must be a string"))
        .to_string()
}

/// The `event` tags of a captured stream, in order — the canonical shape a golden Flow asserts.
#[must_use]
pub fn event_tags(events: &[Event]) -> Vec<String> {
    events.iter().map(event_tag).collect()
}

/// The ordered `(phase, hook-kind)` pairs of the hook events in a captured stream — `phase` is
/// `"start"` or `"finish"`; other events are filtered out.
#[must_use]
pub fn hook_sequence(events: &[Event]) -> Vec<(&'static str, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::HookStart { name } => Some(("start", name.clone())),
            Event::HookFinish { name, .. } => Some(("finish", name.clone())),
            _ => None,
        })
        .collect()
}

/// An all-empty [`Scope`] borrowing one shared empty object — the operand scope for interpolation
/// and matcher cases whose values are inline literals.
#[must_use]
pub fn empty_scope(empty: &Value) -> Scope<'_> {
    Scope {
        inputs: empty,
        env: empty,
        secrets: empty,
        tasks: empty,
        item: None,
        case: None,
        output: None,
        matrix: empty,
    }
}
