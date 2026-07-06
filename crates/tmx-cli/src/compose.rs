//! The composition root — the *only* place concrete adapter types are named
//! (02 §Composition root, 07 §Implementation layout).
//!
//! [`Composed`] owns one instance of every driven adapter and hands the core its port bundle. Wiring
//! lives here and nowhere else: the use cases and the runner see only `dyn Port` trait objects, so the
//! same core runs against these built-in adapters or the testkit fakes without change. Task 17 wires
//! the `exec`/`assert` path — the real process runner, source loader, reference resolver, schema
//! validator, system clock, UUIDv7 id generator, `env`/`file` secret resolver (extended in task 24
//! with the `file` source and the provider trait seam), and the stderr progress
//! reporter — plus the serial scheduler (used once `map` fan-out lands, task 18); task 20 adds the
//! real `reqwest` HTTP client for `fetch` and task 21 the real [`LocalFileSystem`] for `file`; task
//! 22 wires the real `S3ObjectStore` for `store` behind the opt-in `store` Cargo feature (the
//! denying stub stands in when it is off), and task 23 wires the real `ChatCompletionsModel` for
//! `chat-completion` (and the `llmRubric` scorer) behind the opt-in `chat` Cargo feature (the denying
//! stub stands in when it is off). The capability check
//! ([`available_capabilities`](Composed::available_capabilities)) advertises only the ports that are
//! real, so a Flow needing a stubbed one fails preflight up front rather than at the stub.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "chat")]
use tmx_adapters::chat::ChatCompletionsModel;
use tmx_adapters::clock::SystemClock;
#[cfg(not(feature = "chat"))]
use tmx_adapters::deny::DenyingChatModel;
#[cfg(not(feature = "store"))]
use tmx_adapters::deny::DenyingObjectStore;
use tmx_adapters::fs::LocalFileSystem;
use tmx_adapters::http::ReqwestHttpClient;
use tmx_adapters::idgen::Uuidv7Generator;
use tmx_adapters::loader::FileSourceLoader;
use tmx_adapters::process::OsProcessRunner;
use tmx_adapters::resolve::FileReferenceResolver;
use tmx_adapters::runstore::{LocalRunStore, StoringSink};
use tmx_adapters::scheduler::SerialScheduler;
use tmx_adapters::secret::BuiltinSecretResolver;
use tmx_adapters::sink::{Format, ReporterSink};
#[cfg(feature = "store")]
use tmx_adapters::store::S3ObjectStore;
use tmx_adapters::validate::JsonSchemaValidator;

use tmx_core::error::RunError;
use tmx_core::ports::driven::EventSink;
use tmx_core::usecases::EngineRunFlow;
use tmx_core::{
    AvailableCapabilities, CancelToken, Capability, PipelineRunner, Ports, PreflightPorts,
    RunConfig,
};

/// The wired set of built-in adapters — one owner per driven port, plus the always-on infrastructure
/// adapters (clock, id generator, scheduler).
///
/// Constructed per run (the reference resolver is rooted at the Flow's own directory, so references
/// resolve relative to the referring document, 03 §Reference resolution). Owns every adapter so the
/// borrowed [`Ports`] bundle it lends the core outlives the run.
pub struct Composed {
    process: OsProcessRunner,
    http: ReqwestHttpClient,
    file: LocalFileSystem,
    // Real S3-compatible object store when the `store` feature is on, else the denying stub. The
    // capability check advertises `store` as present only in the former case.
    #[cfg(feature = "store")]
    store: S3ObjectStore,
    #[cfg(not(feature = "store"))]
    store: DenyingObjectStore,
    // Real ChatCompletions model when the `chat` feature is on, else the denying stub. The capability
    // check advertises `chat` as present only in the former case.
    #[cfg(feature = "chat")]
    chat: ChatCompletionsModel,
    #[cfg(not(feature = "chat"))]
    chat: DenyingChatModel,
    clock: SystemClock,
    // The event sink handed to the core: the composite streaming reporter (always-on stderr progress
    // plus the `--format`-selected stdout event stream), optionally teed through the run store so each
    // masked event is also persisted (`StoringSink`). Boxed because the concrete type varies with
    // whether the run is recorded; the `json`/`pretty` final-state rendering is a terminal step the run
    // command performs after the loop, not part of this streaming sink.
    events: Box<dyn EventSink>,
    secrets: BuiltinSecretResolver,
    schema: JsonSchemaValidator,
    references: FileReferenceResolver,
    loader: FileSourceLoader,
    ids: Uuidv7Generator,
    // Built now for the default `concurrency: 1` path; wired into `map`/`eval` fan-out in task 18.
    // The sequential runner takes no `Scheduler` (its `Ports` bundle has no scheduler field), so the
    // composed handle is not read on the task-17 run path — `allow(dead_code)` records that it is
    // deliberately composed ahead of the task-18 fan-out that consumes it, not accidentally unused.
    #[allow(dead_code)]
    scheduler: SerialScheduler,
    // The run's cancellation token (task 29): threaded into every adapter call through `ports()`. A
    // fresh `Composed` owns a never-triggered token, so a run wires nothing until `with_cancel` hands
    // it the token the CLI's `--timeout`/SIGINT driver triggers.
    cancel: CancelToken,
    // A separate, never-triggered token for best-effort teardown (`ports()` vs `teardown_ports()`):
    // provider `clean`/`destroy` after a *cancelled* run must complete, so it runs through a token the
    // cancellation never touches, rather than the run's own (now hard-cancelled) `cancel`.
    teardown: CancelToken,
}

impl Composed {
    /// Wire the adapter set, rooting reference resolution at `base_dir` (the Flow's directory) and
    /// building the streaming reporter for the resolved `format`/`color`. Fails only if the embedded
    /// JSON Schema fails to compile — a typed [`RunError`], surfaced up front.
    ///
    /// When `run_store` is `Some`, the streaming reporter is teed through it so each masked event is
    /// also persisted to `./.tmx/runs/`; when `None` (`--no-store`), only the reporter runs and nothing
    /// is recorded.
    ///
    /// # Errors
    ///
    /// Returns the [`JsonSchemaValidator`] compile error if the embedded data-model schema is invalid.
    pub fn new(
        base_dir: PathBuf,
        format: Format,
        color: bool,
        run_store: Option<Arc<LocalRunStore>>,
    ) -> Result<Self, RunError> {
        let reporter = ReporterSink::for_format(format, color);
        // Tee the reporter through the store only when this run is recorded; otherwise the plain
        // reporter is the sink and no event is persisted.
        let events: Box<dyn EventSink> = match &run_store {
            Some(store) => Box::new(StoringSink::new(Box::new(reporter), Arc::clone(store))),
            None => Box::new(reporter),
        };
        Ok(Self {
            process: OsProcessRunner::new(),
            http: ReqwestHttpClient::new()?,
            file: LocalFileSystem::new(),
            #[cfg(feature = "store")]
            store: S3ObjectStore::from_env()?,
            #[cfg(not(feature = "store"))]
            store: DenyingObjectStore,
            #[cfg(feature = "chat")]
            chat: ChatCompletionsModel::from_env()?,
            #[cfg(not(feature = "chat"))]
            chat: DenyingChatModel,
            clock: SystemClock::new(),
            events,
            secrets: BuiltinSecretResolver::new(),
            schema: JsonSchemaValidator::new()?,
            references: FileReferenceResolver::new(base_dir),
            loader: FileSourceLoader::new(),
            ids: Uuidv7Generator::new(),
            scheduler: SerialScheduler::new(),
            cancel: CancelToken::new(),
            teardown: CancelToken::new(),
        })
    }

    /// Install the run's cancellation `token` (the one the CLI's `--timeout`/SIGINT driver triggers),
    /// so every adapter call `ports()` lends the core is threaded with it. Without this, a `Composed`
    /// carries a never-triggered token and cancellation is inert (the shape `tmx env`/tests want).
    #[must_use]
    pub fn with_cancel(mut self, token: CancelToken) -> Self {
        self.cancel = token;
        self
    }

    /// The full driven-port bundle the runner is generic over — every port as a `&dyn` handle.
    #[must_use]
    pub fn ports(&self) -> Ports<'_> {
        Ports {
            process: &self.process,
            http: &self.http,
            file: &self.file,
            store: &self.store,
            chat: &self.chat,
            clock: &self.clock,
            events: self.events.as_ref(),
            secrets: &self.secrets,
            schema: &self.schema,
            reference_resolver: &self.references,
            source_loader: &self.loader,
            cancel: &self.cancel,
        }
    }

    /// The port bundle for best-effort teardown — identical to [`ports`](Self::ports) but threaded
    /// with the never-triggered `teardown` token, so provider `clean`/`destroy` after a *cancelled*
    /// run runs to completion instead of being insta-cancelled by the run's own token.
    #[must_use]
    pub fn teardown_ports(&self) -> Ports<'_> {
        Ports {
            cancel: &self.teardown,
            ..self.ports()
        }
    }

    /// The three ports preflight orchestrates over (resolve → load → validate).
    #[must_use]
    pub fn preflight_ports(&self) -> PreflightPorts<'_> {
        PreflightPorts {
            reference_resolver: &self.references,
            source_loader: &self.loader,
            schema: &self.schema,
        }
    }

    /// The effecting capabilities that are wired and *real* in this build: `exec`/`run` via the
    /// process runner, `fetch` via the `reqwest` HTTP client, `file` via the local filesystem, and
    /// structured secrets via the `env`/`file` resolver. `store` is real only when the `store` Cargo feature
    /// wires the S3-compatible object store, and `chat` only when the `chat` Cargo feature wires the
    /// ChatCompletions model; otherwise each is a denying stub. An unwired capability is advertised as
    /// **absent** — a Flow needing one fails the capability check up front (03 §Capability check)
    /// rather than reaching a stub.
    #[must_use]
    pub fn available_capabilities(&self) -> AvailableCapabilities {
        let caps = AvailableCapabilities::none()
            .with(Capability::Process)
            .with(Capability::Http)
            .with(Capability::File)
            .with(Capability::Secret)
            // The `EnvironmentProvider` port is wired unconditionally (task 25): both the
            // `BinaryProvider` and `FlowProvider` adapters are always compiled, so a Flow declaring an
            // `environment.provider` clears the capability check rather than failing preflight.
            .with(Capability::Provider);
        #[cfg(feature = "store")]
        let caps = caps.with(Capability::Store);
        #[cfg(feature = "chat")]
        let caps = caps.with(Capability::Chat);
        caps
    }

    /// The id generator handle (a run id is minted once, up front, outside the port bundle).
    #[must_use]
    pub fn ids(&self) -> &Uuidv7Generator {
        &self.ids
    }

    /// The serial scheduler handle — the fan-out seam wired into `map`/`eval` in task 18. Held here so
    /// the concurrency port is composed alongside the rest, not bolted on later. Returned as the
    /// concrete type because [`Scheduler`](tmx_core::ports::driven::Scheduler)'s generic method makes
    /// it non-`dyn`-compatible; it is used behind a generic bound, never as `dyn`.
    #[allow(dead_code)] // consumed by the task-18 fan-out path; exercised by this module's tests
    #[must_use]
    pub fn scheduler(&self) -> &SerialScheduler {
        &self.scheduler
    }

    /// Build the `RunFlow` use case (the reference-driven load → resolve → run → mask pipeline) over
    /// this adapter bundle and engine `config`.
    #[must_use]
    pub fn run_flow(&self, config: RunConfig) -> EngineRunFlow<'_> {
        EngineRunFlow::new(self.ports(), &self.ids, config)
    }

    /// A bare [`PipelineRunner`] over `config` — used to execute an already-preflighted, assembled
    /// Flow (a directory / folder layout has no single file reference to drive the `RunFlow` use case).
    #[must_use]
    pub fn runner(&self, config: RunConfig) -> PipelineRunner {
        PipelineRunner::new(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use tmx_core::ports::driven::Scheduler;

    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    #[test]
    fn composes_the_adapter_bundle_and_advertises_only_real_capabilities() {
        let composed = Composed::new(PathBuf::from("."), Format::Json, false, None)
            .expect("the embedded schema compiles");
        // The full port bundle is buildable — every driven port has a wired handle.
        let ports = composed.ports();
        let _ = ports.process; // touch the bundle so the borrow is exercised

        // Only the real effecting ports are advertised; the stubbed executors are absent, so a Flow
        // needing them fails the capability check rather than reaching a denying stub.
        let caps = composed.available_capabilities();
        assert!(caps.has(Capability::Process), "exec/run is wired and real");
        assert!(caps.has(Capability::Secret), "env secrets are wired");
        assert!(
            caps.has(Capability::Provider),
            "the environment provider port is wired unconditionally"
        );
        assert!(caps.has(Capability::Http), "fetch is wired and real");
        assert!(
            caps.has(Capability::File),
            "file is wired to the local filesystem and real"
        );
        #[cfg(not(feature = "store"))]
        assert!(
            !caps.has(Capability::Store),
            "store is a denying stub, not real without the `store` feature"
        );
        #[cfg(feature = "store")]
        assert!(
            caps.has(Capability::Store),
            "store is wired to the S3 object store and real with the `store` feature"
        );
        #[cfg(not(feature = "chat"))]
        assert!(
            !caps.has(Capability::Chat),
            "chat is a denying stub, not real without the `chat` feature"
        );
        #[cfg(feature = "chat")]
        assert!(
            caps.has(Capability::Chat),
            "chat is wired to the ChatCompletions model and real with the `chat` feature"
        );

        // The scheduler is composed now (wired into fan-out in task 18) and is a usable handle: a
        // trivial serial fan-out runs through it, proving it is wired, not merely constructed.
        let results = block_on_ready(
            composed
                .scheduler()
                .run_indexed(2, 1, |i| async move { Ok::<u32, RunError>(i) }),
        );
        assert_eq!(results.len(), 2, "the composed scheduler runs both indices");
    }
}
