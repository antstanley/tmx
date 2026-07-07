#![forbid(unsafe_code)]
//! `tmx-testkit` — the in-memory fake adapters.
//!
//! One deterministic fake per driven port, mirroring `tmx-adapters` but with no real I/O: a
//! strictly serial `Scheduler`, a frozen `Clock`, a seeded `IdGenerator`, and recording stand-ins
//! for the process, HTTP, chat, filesystem, object-store, run-store, secret, source-loading, schema,
//! environment-provider, and event-sink ports. The core's unit tests, the workspace conformance
//! suite, and downstream embedders inject this one shared fake set instead of the built-in adapters
//! — the determinism payoff of the hexagon.
//!
//! Depends on `tmx-core` and `tmx-schema` only (plus the pure `serde_json`/`indexmap` data crates
//! and the trait-support `async-trait` macro) — no `tokio`, no `reqwest`, no I/O crate — so it stays
//! inside the same purity boundary as the core it fakes (the `cargo tree` purity check covers it
//! too).
//!
//! ## The determinism seam
//!
//! Three fakes make a run reproducible: [`SerialScheduler`] runs fan-out strictly serially in index
//! order, [`FixedClock`] freezes the wall clock (advanceable only on demand), and
//! [`SeededIdGenerator`] mints a fixed UUIDv7 sequence. Two fresh [`Fakes`] bundles built from the
//! same seeds drive byte-identical event streams and run ids — the property the conformance suite
//! asserts on.

pub mod chat;
pub mod clock;
pub mod environment;
pub mod fs;
pub mod http;
pub mod idgen;
pub mod process;
pub mod scheduler;
pub mod schema;
pub mod secret;
pub mod sink;
pub mod source;
pub mod store;

pub use chat::FakeChatModel;
pub use clock::FixedClock;
pub use environment::FakeEnvironmentProvider;
pub use fs::MemFileSystem;
pub use http::FakeHttpClient;
pub use idgen::SeededIdGenerator;
pub use process::RecordingProcessRunner;
pub use scheduler::SerialScheduler;
pub use schema::FakeSchemaValidator;
pub use secret::FakeSecretResolver;
pub use sink::RecordingEventSink;
pub use source::{FakeReferenceResolver, FakeSourceLoader};
pub use store::{MemObjectStore, MemRunStore};

/// One deterministic fake per driven port, assembled as a single injectable bundle.
///
/// The composition root of a test: build it once, hand its fields to a use case (each satisfies its
/// `tmx-core` driven-port trait), and drive a Flow. Two bundles built with [`Fakes::new`] share the
/// same seeds, so replaying the same sequence over each yields byte-identical output. Fields are
/// public so a test both injects a port and inspects its recording afterwards.
#[derive(Debug, Default)]
pub struct Fakes {
    /// The strictly-serial, index-ordered scheduler (the fan-out determinism seam).
    pub scheduler: SerialScheduler,
    /// The frozen, step-advanceable clock.
    pub clock: FixedClock,
    /// The seeded UUIDv7 id generator.
    pub ids: SeededIdGenerator,
    /// The scripted, recording process runner (`exec`/`run`).
    pub process: RecordingProcessRunner,
    /// The canned-response, recording HTTP client (`fetch`).
    pub http: FakeHttpClient,
    /// The canned-completion, recording chat model (`chat-completion`, `llmRubric`).
    pub chat: FakeChatModel,
    /// The in-memory filesystem (`file`).
    pub fs: MemFileSystem,
    /// The in-memory object store (`store`).
    pub object_store: MemObjectStore,
    /// The in-memory run store.
    pub run_store: MemRunStore,
    /// The capturing event sink (reporters).
    pub event_sink: RecordingEventSink,
    /// The seeded secret resolver.
    pub secrets: FakeSecretResolver,
    /// The seeded source loader.
    pub source_loader: FakeSourceLoader,
    /// The seeded reference resolver.
    pub reference_resolver: FakeReferenceResolver,
    /// The scripted schema validator.
    pub schema_validator: FakeSchemaValidator,
    /// The scripted environment provider.
    pub environment_provider: FakeEnvironmentProvider,
}

impl Fakes {
    /// Assemble the full deterministic fake port set with default seeds. Two bundles from `new`
    /// behave identically until one is scripted or stepped.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use indexmap::IndexMap;
    use serde_json::json;

    use super::*;
    use tmx_core::mask::Masker;
    use tmx_core::ports::driven::{
        ArtifactKind, ChatModel, Clock, EnvironmentProvider, EventSink, FileOp, FileResult,
        FileSystem, HttpClient, HttpRequest, IdGenerator, ObjectStore, ProcessKind, ProcessRunner,
        ProcessSpec, ProviderMethod, ReferenceResolver, RunStore, Scheduler, SchemaValidator,
        SecretResolver, SourceKind, SourceLoader, StoreOp, StoreResult,
    };
    use tmx_core::{Event, Milliseconds, RunError, RunId, RunStatus};
    use tmx_schema::SecretSource;

    /// Drive an immediately-ready future to completion with a no-op waker — the same purity-preserving
    /// pattern `tmx-core`'s tests use, so no async runtime is pulled in.
    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a fake future must be immediately ready"),
        }
    }

    fn sample_spec() -> ProcessSpec {
        ProcessSpec {
            kind: ProcessKind::Exec,
            command: "echo hi".to_string(),
            language: None,
            args: Vec::new(),
            env: IndexMap::new(),
            cwd: None,
            stdin: None,
            timeout: None,
        }
    }

    /// A throwaway driving use case, generic over three driven ports: it mints two ids, emits a
    /// bracketed event stream, and runs one process — proving the bundle's fakes are injectable
    /// across generic port bounds and type-check as their `tmx-core` traits.
    async fn drive<I, S, P>(ids: &I, sink: &S, process: &P) -> Vec<RunId>
    where
        I: IdGenerator,
        S: EventSink,
        P: ProcessRunner,
    {
        let run_id = ids.new_run_id();
        let next_id = ids.new_run_id();
        // The `EventSink` port accepts a proven-routed `Masked<Event>`; seal each event through a
        // Masker exactly as the real runner does before emitting it.
        let masker = Masker::new();
        sink.emit(&masker.redact_event(&Event::RunStart {
            id: run_id.clone(),
            flow: "deploy".to_string(),
        }))
        .await
        .expect("sink accepts run.start");
        sink.emit(&masker.redact_event(&Event::TaskStart {
            name: "build".to_string(),
        }))
        .await
        .expect("sink accepts task.start");
        let out = process.run(sample_spec()).await.expect("process runs");
        sink.emit(&masker.redact_event(&Event::TaskFinish {
            name: "build".to_string(),
            status: tmx_core::TaskStatus::Ok,
            ms: out.ms,
            output: None,
        }))
        .await
        .expect("sink accepts task.finish");
        sink.emit(&masker.redact_event(&Event::RunFinish {
            id: run_id.clone(),
            status: RunStatus::Ok,
            ms: Milliseconds(0),
        }))
        .await
        .expect("sink accepts run.finish");
        vec![run_id, next_id]
    }

    #[test]
    fn the_bundle_injects_into_a_use_case_and_records_the_run() {
        // O1: the bundle's fakes satisfy their driven-port traits and inject into a use case. Drive
        // one, then inspect the recordings the injected fakes captured.
        let fakes = Fakes::new();
        let ids = block_on_ready(drive(&fakes.ids, &fakes.event_sink, &fakes.process));
        assert_eq!(ids.len(), 2, "the use case minted two ids");
        assert_eq!(
            fakes.event_sink.events().len(),
            4,
            "the recording sink captured every emitted event"
        );
        assert_eq!(
            fakes.process.calls().len(),
            1,
            "the recording process runner captured the one invocation"
        );
        assert_eq!(
            fakes.process.calls()[0].command,
            "echo hi",
            "the captured spec carries the exact command the use case ran"
        );
    }

    #[test]
    fn every_non_generic_driven_port_is_object_safe_as_dyn() {
        // The composition root injects each driven port as `dyn Port`. Binding each fake behind its
        // trait object forces the object-safety check. Scheduler is absent by design (generic method).
        let _: Box<dyn ProcessRunner> = Box::new(RecordingProcessRunner::new());
        let _: Box<dyn HttpClient> = Box::new(FakeHttpClient::new());
        let _: Box<dyn ChatModel> = Box::new(FakeChatModel::new());
        let _: Box<dyn FileSystem> = Box::new(MemFileSystem::new());
        let _: Box<dyn ObjectStore> = Box::new(MemObjectStore::new());
        let _: Box<dyn RunStore> = Box::new(MemRunStore::new());
        let _: Box<dyn EventSink> = Box::new(RecordingEventSink::new());
        let _: Box<dyn SecretResolver> = Box::new(FakeSecretResolver::new());
        let _: Box<dyn SourceLoader> = Box::new(FakeSourceLoader::new());
        let _: Box<dyn ReferenceResolver> = Box::new(FakeReferenceResolver::new());
        let _: Box<dyn EnvironmentProvider> = Box::new(FakeEnvironmentProvider::new());
        let _: Box<dyn SchemaValidator> = Box::new(FakeSchemaValidator::new());
        let _: Box<dyn Clock> = Box::new(FixedClock::new());
        let boxed_ids: Box<dyn IdGenerator> = Box::new(SeededIdGenerator::new());
        assert!(
            RunId::new(boxed_ids.new_run_id().as_str()).is_ok(),
            "a boxed IdGenerator mints a well-formed id via its vtable"
        );
    }

    #[test]
    fn serial_scheduler_returns_index_order_for_a_shuffled_completion_order() {
        // O2: the work's payloads are a shuffle; a completion-ordered collector would sort them, but
        // the scheduler keys results by submission index, so the shuffle is preserved in order.
        let scheduler = SerialScheduler::new();
        let shuffled = [40u32, 10, 30, 0, 20];
        let results = block_on_ready(scheduler.run_indexed(
            shuffled.len() as u32,
            2,
            |index| async move { Ok::<u32, RunError>(shuffled[index as usize]) },
        ));
        let values: Vec<u32> = results
            .into_iter()
            .map(|r| r.expect("each unit succeeds"))
            .collect();
        assert_eq!(
            values,
            vec![40, 10, 30, 0, 20],
            "results emerge in submission-index order, carrying the shuffle"
        );

        // Negative-space companion: expecting completion (value-sorted) order is wrong — the index
        // order is NOT the sorted order, so an index-agnostic expectation fails.
        let mut sorted = values.clone();
        sorted.sort_unstable();
        assert_ne!(
            values, sorted,
            "index order differs from completion (value-sorted) order, proving the ordering is by index"
        );
    }

    #[test]
    fn serial_scheduler_returns_exactly_count_results_and_handles_zero_width() {
        let scheduler = SerialScheduler::new();
        let full =
            block_on_ready(scheduler.run_indexed(3, 1, |i| async move { Ok::<u32, RunError>(i) }));
        assert_eq!(full.len(), 3, "one result per index");
        assert_eq!(
            full.into_iter().map(|r| r.expect("ok")).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "in index order"
        );

        let empty =
            block_on_ready(scheduler.run_indexed(0, 4, |i| async move { Ok::<u32, RunError>(i) }));
        assert!(empty.is_empty(), "a zero-width fan-out returns no results");
    }

    #[test]
    #[should_panic(expected = "concurrency must be at least one unit")]
    fn serial_scheduler_rejects_zero_concurrency() {
        // Negative space: a zero concurrency budget violates the port contract and is asserted, not
        // silently accepted.
        let scheduler = SerialScheduler::new();
        let _ =
            block_on_ready(scheduler.run_indexed(1, 0, |i| async move { Ok::<u32, RunError>(i) }));
    }

    #[test]
    fn fixed_clock_and_seeded_ids_produce_identical_sequences_across_two_runs() {
        // O2: two fresh clocks stepped identically read identically; two fresh generators with the
        // same seed mint the same id sequence.
        fn clock_sequence() -> Vec<(String, u64)> {
            let clock = FixedClock::new();
            let mut seq = Vec::new();
            for step in 0..5u64 {
                clock.advance_ms(step);
                seq.push((clock.now().as_str().to_string(), clock.now_ms().0));
            }
            seq
        }
        assert_eq!(
            clock_sequence(),
            clock_sequence(),
            "two fresh clocks stepped the same way read identically"
        );

        fn id_sequence() -> Vec<String> {
            let ids = SeededIdGenerator::new();
            (0..8)
                .map(|_| ids.new_run_id().as_str().to_string())
                .collect()
        }
        let first = id_sequence();
        let second = id_sequence();
        assert_eq!(
            first, second,
            "two fresh generators with the same seed mint identical id sequences"
        );
        // The sequence is genuinely a sequence, not one id repeated.
        let mut distinct = first.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            first.len(),
            "each id in the sequence is distinct"
        );
    }

    #[test]
    fn every_generated_id_is_a_valid_uuid_v7_and_monotonically_ordered() {
        // Proves the `unreachable!` branch in `SeededIdGenerator::new_run_id` is dead: every id over
        // a long run is a well-formed UUIDv7, and the sequence is lexically increasing (UUIDv7's
        // chronological-sort property the RunStore relies on).
        let ids = SeededIdGenerator::new();
        let mut previous: Option<String> = None;
        for _ in 0..1000 {
            let id = ids.new_run_id();
            let text = id.as_str().to_string();
            assert!(
                RunId::new(&text).is_ok(),
                "generated id {text} matches the UUIDv7 pattern"
            );
            if let Some(prev) = &previous {
                assert!(
                    prev.as_str() < text.as_str(),
                    "the id sequence is strictly lexically increasing: {prev} < {text}"
                );
            }
            previous = Some(text);
        }
    }

    #[test]
    fn mem_filesystem_round_trips_and_reports_missing_paths() {
        let fs = MemFileSystem::new();
        block_on_ready(fs.op(FileOp::Write {
            path: "/a.txt".to_string(),
            contents: b"hello".to_vec(),
        }))
        .expect("write succeeds");
        let read = block_on_ready(fs.op(FileOp::Read {
            path: "/a.txt".to_string(),
            encoding: None,
        }))
        .expect("read succeeds");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"hello".to_vec()
            },
            "the write round-trips through memory"
        );
        assert!(fs.contains("/a.txt"), "the seeded path is present");

        // Negative space: reading a missing path is a typed RunError, not a panic.
        let missing = block_on_ready(fs.op(FileOp::Read {
            path: "/nope.txt".to_string(),
            encoding: None,
        }))
        .expect_err("a missing path is an error");
        assert_eq!(
            missing.code, "file_not_found",
            "the missing-file error carries its stable code"
        );
    }

    #[test]
    fn mem_object_store_lists_in_sorted_order_and_errors_on_missing_get() {
        let store = MemObjectStore::new()
            .with_object("b/2", b"two".to_vec())
            .with_object("a/1", b"one".to_vec())
            .with_object("b/1", b"three".to_vec());
        let listed = block_on_ready(store.op(
            StoreOp::List {
                prefix: "b/".to_string(),
            },
            None,
        ))
        .expect("list succeeds");
        assert_eq!(
            listed,
            StoreResult::List {
                keys: vec!["b/1".to_string(), "b/2".to_string()]
            },
            "listing is prefix-filtered and deterministically sorted"
        );

        // Negative space: getting an absent key is a typed error.
        let missing = block_on_ready(store.op(
            StoreOp::Get {
                key: "missing".to_string(),
            },
            None,
        ))
        .expect_err("a missing object is an error");
        assert_eq!(
            missing.code, "object_not_found",
            "the error names the missing object"
        );
    }

    #[test]
    fn mem_run_store_saves_lists_and_prunes_by_time() {
        let store = MemRunStore::new();
        let early = RunId::new("018f8c7e-0000-7def-8123-456789abcdef").expect("valid id");
        let late = RunId::new("018f8c7e-9b2a-7def-8123-456789abcdef").expect("valid id");
        for (id, started) in [
            (&early, "2026-01-01T00:00:00Z"),
            (&late, "2026-12-31T00:00:00Z"),
        ] {
            block_on_ready(store.save(&tmx_core::RunRecord {
                id: id.clone(),
                flow: Some("deploy".to_string()),
                status: RunStatus::Ok,
                started_at: tmx_core::Timestamp::new(started),
                finished_at: None,
                ms: None,
                final_state: None,
                results: Vec::new(),
            }))
            .expect("save succeeds");
        }
        let listed = block_on_ready(store.list()).expect("list succeeds");
        assert_eq!(
            listed,
            vec![early.clone(), late.clone()],
            "runs list in UUIDv7 order"
        );

        block_on_ready(store.append_event(
            &late,
            &Event::TaskStart {
                name: "build".to_string(),
            },
        ))
        .expect("append succeeds");
        assert_eq!(
            store.events_for(&late).len(),
            1,
            "the event log records the append"
        );

        // Prune everything started before mid-2026: only the early run goes.
        let removed =
            block_on_ready(store.prune(&tmx_core::Timestamp::new("2026-06-01T00:00:00Z")))
                .expect("prune succeeds");
        assert_eq!(removed, 1, "exactly the one stale run is pruned");
        assert_eq!(
            block_on_ready(store.list()).expect("list"),
            vec![late],
            "only the recent run survives"
        );
    }

    #[test]
    fn scripted_ports_replay_and_default_deterministically() {
        // Scripted results replay FIFO; an unscripted call falls back to a deterministic default.
        let http = FakeHttpClient::new().with_response(201, b"created".to_vec());
        let scripted = block_on_ready(http.send(HttpRequest {
            method: "POST".to_string(),
            url: "https://example.test".to_string(),
            headers: IndexMap::new(),
            query: IndexMap::new(),
            body: None,
            follow_redirects: true,
            retries: 0,
            timeout: None,
        }))
        .expect("scripted response");
        assert_eq!(scripted.status, 201, "the scripted status replays");
        assert_eq!(scripted.body, b"created", "the scripted body replays");
        assert_eq!(http.requests().len(), 1, "the request was recorded");

        // The queue is now empty: the next call defaults to 200 with an empty body.
        let default = block_on_ready(http.send(HttpRequest {
            method: "GET".to_string(),
            url: "https://example.test/next".to_string(),
            headers: IndexMap::new(),
            query: IndexMap::new(),
            body: None,
            follow_redirects: true,
            retries: 0,
            timeout: None,
        }))
        .expect("default response");
        assert_eq!(
            default.status, 200,
            "an unscripted call defaults deterministically"
        );
    }

    #[test]
    fn secret_resolver_resolves_seeded_and_errors_on_missing() {
        let resolver = FakeSecretResolver::new().with_secret("API_KEY", "s3cr3t");
        let env_source = SecretSource {
            env: Some("API_KEY".to_string()),
            file: None,
            provider: None,
            key: None,
            extra: IndexMap::new(),
        };
        assert_eq!(
            block_on_ready(resolver.resolve(&env_source)).expect("seeded secret resolves"),
            "s3cr3t",
            "the seeded value comes back"
        );

        // Negative space: an unseeded source is a typed resolution error.
        let missing = SecretSource {
            env: Some("NOPE".to_string()),
            file: None,
            provider: None,
            key: None,
            extra: IndexMap::new(),
        };
        let err = block_on_ready(resolver.resolve(&missing)).expect_err("missing secret errors");
        assert_eq!(
            err.code, "secret_not_found",
            "the error names the missing secret"
        );
        assert_eq!(
            err.category,
            tmx_core::ErrorCategory::Resolution,
            "a missing secret is a resolution failure"
        );
    }

    #[test]
    fn source_loader_and_reference_resolver_serve_seeded_and_error_on_missing() {
        let loader = FakeSourceLoader::new().with_source("flow.yaml", json!({ "name": "deploy" }));
        assert_eq!(
            block_on_ready(loader.load("flow.yaml", SourceKind::Yaml))
                .expect("seeded source loads"),
            json!({ "name": "deploy" }),
            "the seeded value loads"
        );
        assert!(
            block_on_ready(loader.load("missing.yaml", SourceKind::Yaml)).is_err(),
            "a missing source is an error"
        );

        let resolver =
            FakeReferenceResolver::new().with_reference("deploy", "flow.yaml", SourceKind::Yaml);
        let resolved =
            block_on_ready(resolver.resolve("deploy")).expect("seeded reference resolves");
        assert_eq!(
            resolved.path, "flow.yaml",
            "the reference resolves to its path"
        );
        assert_eq!(resolved.kind, SourceKind::Yaml, "and its declared kind");
        assert!(
            block_on_ready(resolver.resolve("unknown")).is_err(),
            "a missing reference is an error"
        );
    }

    #[test]
    fn schema_validator_and_environment_provider_behave_deterministically() {
        // The default validator reports valid; a seeded diagnostic drives the invalid path.
        let ok = FakeSchemaValidator::new();
        assert!(
            block_on_ready_sync(ok.validate(&json!({}), ArtifactKind::Flow)).is_empty(),
            "the default validator reports no diagnostics"
        );
        let bad = FakeSchemaValidator::new().with_diagnostic(tmx_core::Diagnostic::new(
            tmx_core::Severity::Error,
            "schema_invalid",
            "seeded failure",
        ));
        assert_eq!(
            block_on_ready_sync(bad.validate(&json!({}), ArtifactKind::Flow)).len(),
            1,
            "a seeded diagnostic is returned"
        );

        let provider = FakeEnvironmentProvider::new();
        let sample_env: tmx_schema::Environment =
            serde_json::from_value(json!({ "name": "test" })).expect("environment deserialises");
        let outcome = block_on_ready(provider.invoke(ProviderMethod::Deploy, &sample_env))
            .expect("deploy succeeds");
        assert_eq!(
            outcome.method,
            ProviderMethod::Deploy,
            "the outcome echoes the method"
        );
        assert_eq!(
            provider.calls(),
            vec![ProviderMethod::Deploy],
            "the provider recorded the lifecycle call"
        );
    }

    /// A sync helper mirroring `block_on_ready` for the sync `SchemaValidator` port — unwraps the
    /// `Result` a validate call returns.
    fn block_on_ready_sync(
        result: Result<Vec<tmx_core::Diagnostic>, RunError>,
    ) -> Vec<tmx_core::Diagnostic> {
        result.expect("validation runs synchronously and does not fault")
    }

    #[test]
    fn two_fresh_bundles_drive_byte_identical_event_streams_and_ids() {
        // O4 (reviewable): construct the bundle, drive the same sequence twice over two fresh
        // bundles, and confirm byte-identical event streams (as ndjson) and byte-identical ids.
        fn run_once() -> (String, Vec<String>) {
            let fakes = Fakes::new();
            let ids = block_on_ready(drive(&fakes.ids, &fakes.event_sink, &fakes.process));
            let stream = fakes
                .event_sink
                .ndjson()
                .expect("the event stream serialises");
            (stream, ids.iter().map(|i| i.as_str().to_string()).collect())
        }
        let (stream_a, ids_a) = run_once();
        let (stream_b, ids_b) = run_once();
        assert_eq!(
            stream_a, stream_b,
            "the two runs emit byte-identical event streams"
        );
        assert_eq!(ids_a, ids_b, "the two runs mint byte-identical ids");
        assert!(!stream_a.is_empty(), "the run actually emitted events");
    }
}
