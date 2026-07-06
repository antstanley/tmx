//! The **port traits** the core owns — the whole edge of the hexagon.
//!
//! Two families, in two submodules:
//!
//! - [`driven`] — every capability the core needs *from* the outside world (a process, an HTTP
//!   client, a filesystem, an object store, a chat model, secret resolution, provider lifecycle, the
//!   run store, event sinks, source loading, reference resolution, schema validation, the clock, the
//!   id generator, the scheduler). Each is implemented by one built-in adapter in `tmx-adapters`.
//! - [`driving`] — one use case per CLI command (`RunFlow`, `ValidateArtifacts`, `LintFlow`,
//!   `InspectFlow`, `ScaffoldFlow`, `FormatArtifact`, `Discover`, `ProvisionEnvironment`,
//!   `ManageProviders`, `QueryRuns`). The `tmx` binary calls these; the composition root wires them.
//!
//! The pure/async boundary is fixed *here*: a driven method is `async` iff it effects the outside
//! world, and sync otherwise ([`driven::Clock`], [`driven::IdGenerator`], [`driven::SchemaValidator`]).
//! Every method returns a [`RunError`](crate::error::RunError), so one typed-error vocabulary spans
//! the whole edge. This module declares the traits only — no adapter or use-case body lives here (they
//! arrive in tasks 06/11+), and the core stays I/O-free (the `cargo tree` purity gate proves it).

pub mod driven;
pub mod driving;

pub use driven::{
    ArtifactKind, ChatModel, ChatRequest, ChatResponse, Clock, EnvironmentProvider, EventSink,
    FileOp, FileResult, FileSystem, HttpClient, HttpRequest, HttpResponse, IdGenerator,
    ObjectStore, ProcessKind, ProcessOutput, ProcessRunner, ProcessSpec, ProviderMethod,
    ProviderOutcome, ReferenceResolver, ResolvedSource, RunStore, Scheduler, SchemaValidator,
    SecretResolver, SourceKind, SourceLoader, StoreOp, StoreResult,
};
pub use driving::{
    Discover, DiscoverKind, FormatArtifact, InspectFlow, Inspection, LintFlow, ManageProviders,
    ProviderOp, ProvisionEnvironment, QueryRuns, RunFlow, RunOptions, RunQuery, ScaffoldFlow,
    ScaffoldLayout, ValidateArtifacts,
};

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use async_trait::async_trait;
    use indexmap::IndexMap;
    use serde_json::{Value, json};
    use tmx_schema::Environment;
    use tmx_schema::context::SecretSource;

    use super::driven::*;
    use super::driving::*;
    use crate::error::{ErrorCategory, RunError};
    use crate::model::{Diagnostic, Event, Milliseconds, RunId, RunRecord, RunStatus, Timestamp};

    const VALID_UUID_V7: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

    /// Drive a future to completion by polling once with a no-op waker.
    ///
    /// The fakes in these tests never yield (no real await point), so their futures — including the
    /// `Pin<Box<dyn Future>>` an `#[async_trait]` method returns — are ready on the first poll. This
    /// keeps `tmx-core`'s tests free of any async-runtime dependency, preserving the purity boundary
    /// (no `tokio`, not even as a dev-dependency). `Waker::noop` is safe, so `#![forbid(unsafe_code)]`
    /// holds.
    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a test fake future must be immediately ready"),
        }
    }

    /// One fake that implements every non-generic driven port, each returning a canned `Ok`. Boxing
    /// it as each `dyn Port` is the object-safety proof the composition root relies on.
    struct OkFake;

    #[async_trait]
    impl ProcessRunner for OkFake {
        async fn run(&self, spec: ProcessSpec) -> Result<ProcessOutput, RunError> {
            let _ = spec;
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: b"out".to_vec(),
                stderr: Vec::new(),
                ms: Milliseconds(1),
            })
        }
    }

    #[async_trait]
    impl HttpClient for OkFake {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RunError> {
            let _ = request;
            Ok(HttpResponse {
                status: 200,
                headers: IndexMap::new(),
                body: b"body".to_vec(),
                ms: Milliseconds(2),
            })
        }
    }

    #[async_trait]
    impl FileSystem for OkFake {
        async fn op(&self, op: FileOp) -> Result<FileResult, RunError> {
            match op {
                FileOp::Read { .. } => Ok(FileResult::Read {
                    contents: b"data".to_vec(),
                }),
                FileOp::Exists { .. } => Ok(FileResult::Exists { exists: true }),
                _ => Ok(FileResult::Done),
            }
        }
    }

    #[async_trait]
    impl ObjectStore for OkFake {
        async fn op(&self, op: StoreOp) -> Result<StoreResult, RunError> {
            match op {
                StoreOp::Get { .. } => Ok(StoreResult::Get {
                    body: b"obj".to_vec(),
                }),
                StoreOp::List { .. } => Ok(StoreResult::List {
                    keys: vec!["k".to_string()],
                }),
                StoreOp::Head { .. } => Ok(StoreResult::Head {
                    exists: true,
                    size_bytes: Some(3),
                }),
                _ => Ok(StoreResult::Done),
            }
        }
    }

    #[async_trait]
    impl ChatModel for OkFake {
        async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, RunError> {
            Ok(ChatResponse {
                content: "hi".to_string(),
                model: request.model,
                prompt_tokens: Some(1),
                completion_tokens: Some(1),
                ms: Milliseconds(3),
            })
        }
    }

    #[async_trait]
    impl SecretResolver for OkFake {
        async fn resolve(&self, source: &SecretSource) -> Result<String, RunError> {
            let _ = source;
            Ok("s3cr3t".to_string())
        }
    }

    #[async_trait]
    impl EnvironmentProvider for OkFake {
        async fn invoke(
            &self,
            method: ProviderMethod,
            environment: &Environment,
        ) -> Result<ProviderOutcome, RunError> {
            let _ = environment;
            Ok(ProviderOutcome {
                method,
                output: json!({ "ok": true }),
                ms: Milliseconds(4),
            })
        }
    }

    #[async_trait]
    impl RunStore for OkFake {
        async fn save(&self, record: &RunRecord) -> Result<(), RunError> {
            let _ = record;
            Ok(())
        }
        async fn append_event(&self, id: &RunId, event: &Event) -> Result<(), RunError> {
            let _ = (id, event);
            Ok(())
        }
        async fn list(&self) -> Result<Vec<RunId>, RunError> {
            Ok(Vec::new())
        }
        async fn get(&self, id: &RunId) -> Result<Option<RunRecord>, RunError> {
            let _ = id;
            Ok(None)
        }
        async fn prune(&self, cutoff: &Timestamp) -> Result<u32, RunError> {
            let _ = cutoff;
            Ok(0)
        }
        async fn remove(&self, id: &RunId) -> Result<(), RunError> {
            let _ = id;
            Ok(())
        }
    }

    #[async_trait]
    impl EventSink for OkFake {
        async fn emit(&self, event: &crate::mask::Masked<Event>) -> Result<(), RunError> {
            let _ = event;
            Ok(())
        }
    }

    #[async_trait]
    impl SourceLoader for OkFake {
        async fn load(&self, path: &str, kind: SourceKind) -> Result<Value, RunError> {
            let _ = (path, kind);
            Ok(json!({ "loaded": true }))
        }
    }

    #[async_trait]
    impl ReferenceResolver for OkFake {
        async fn resolve(&self, reference: &str) -> Result<ResolvedSource, RunError> {
            Ok(ResolvedSource {
                path: reference.to_string(),
                kind: SourceKind::Yaml,
            })
        }
    }

    impl SchemaValidator for OkFake {
        fn validate(
            &self,
            instance: &Value,
            kind: ArtifactKind,
        ) -> Result<Vec<Diagnostic>, RunError> {
            let _ = (instance, kind);
            Ok(Vec::new())
        }
        fn validate_produces(
            &self,
            output: &Value,
            schema: &Value,
        ) -> Result<Vec<Diagnostic>, RunError> {
            let _ = (output, schema);
            Ok(Vec::new())
        }
    }

    impl Clock for OkFake {
        fn now(&self) -> Timestamp {
            Timestamp::new("2026-07-05T00:00:00Z")
        }
        fn now_ms(&self) -> Milliseconds {
            Milliseconds(42)
        }
    }

    impl IdGenerator for OkFake {
        fn new_run_id(&self) -> RunId {
            RunId::new(VALID_UUID_V7).expect("the canned UUIDv7 is valid")
        }
    }

    /// A fake whose one method fails — the negative-space proof that a port propagates a typed
    /// [`RunError`] to the caller, category intact, rather than panicking.
    struct ErrFake;

    #[async_trait]
    impl ProcessRunner for ErrFake {
        async fn run(&self, _spec: ProcessSpec) -> Result<ProcessOutput, RunError> {
            Err(RunError::run_failure("spawn_failed", "no such binary").with_task("build"))
        }
    }

    /// A serial [`Scheduler`] fake: runs `make(i)` for `i in 0..count` strictly in order. Proves the
    /// generic port is usable behind a generic bound and honours its index-order + length contract.
    struct SerialScheduler;

    impl Scheduler for SerialScheduler {
        async fn run_indexed<T, F, Fut>(
            &self,
            count: u32,
            concurrency: u32,
            make: F,
        ) -> Vec<Result<T, RunError>>
        where
            T: Send,
            F: Fn(u32) -> Fut + Send + Sync,
            Fut: Future<Output = Result<T, RunError>> + Send,
        {
            assert!(concurrency >= 1, "concurrency must be at least one unit");
            let mut out = Vec::with_capacity(count as usize);
            for index in 0..count {
                out.push(make(index).await);
            }
            assert_eq!(out.len(), count as usize, "one result per index, in order");
            out
        }
    }

    /// One fake that implements every driving use case, each returning a canned `Ok`. Boxing it as
    /// each `dyn UseCase` is the object-safety proof the CLI relies on to hold a use case behind a
    /// trait object.
    struct UseCaseFake;

    #[async_trait]
    impl RunFlow for UseCaseFake {
        async fn run(
            &self,
            reference: &str,
            inputs: Value,
            options: RunOptions,
        ) -> Result<RunRecord, RunError> {
            let _ = (inputs, options);
            Ok(RunRecord {
                id: RunId::new(VALID_UUID_V7).expect("valid id"),
                flow: Some(reference.to_string()),
                status: RunStatus::Ok,
                started_at: Timestamp::new("2026-07-05T00:00:00Z"),
                finished_at: Some(Timestamp::new("2026-07-05T00:00:01Z")),
                ms: Some(Milliseconds(1000)),
                final_state: None,
                results: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl ValidateArtifacts for UseCaseFake {
        async fn validate(&self, paths: Vec<String>) -> Result<Vec<Diagnostic>, RunError> {
            let _ = paths;
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl LintFlow for UseCaseFake {
        async fn lint(&self, reference: &str) -> Result<Vec<Diagnostic>, RunError> {
            let _ = reference;
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl InspectFlow for UseCaseFake {
        async fn inspect(&self, reference: &str) -> Result<Inspection, RunError> {
            let _ = reference;
            Ok(Inspection {
                view: json!({ "tasks": [] }),
            })
        }
    }

    #[async_trait]
    impl ScaffoldFlow for UseCaseFake {
        async fn scaffold(
            &self,
            name: &str,
            template: &str,
            layout: ScaffoldLayout,
        ) -> Result<Vec<String>, RunError> {
            let _ = (name, template, layout);
            Ok(vec!["flow.yaml".to_string()])
        }
    }

    #[async_trait]
    impl FormatArtifact for UseCaseFake {
        async fn format(&self, path: &str, to: Option<SourceKind>) -> Result<String, RunError> {
            let _ = (path, to);
            Ok("formatted: true\n".to_string())
        }
    }

    #[async_trait]
    impl Discover for UseCaseFake {
        async fn discover(
            &self,
            kind: DiscoverKind,
            reference: Option<&str>,
        ) -> Result<Value, RunError> {
            let _ = (kind, reference);
            Ok(json!([]))
        }
    }

    #[async_trait]
    impl ProvisionEnvironment for UseCaseFake {
        async fn provision(
            &self,
            reference: &str,
            method: ProviderMethod,
        ) -> Result<ProviderOutcome, RunError> {
            let _ = reference;
            Ok(ProviderOutcome {
                method,
                output: json!({ "ok": true }),
                ms: Milliseconds(5),
            })
        }
    }

    #[async_trait]
    impl ManageProviders for UseCaseFake {
        async fn manage(&self, op: ProviderOp) -> Result<Value, RunError> {
            let _ = op;
            Ok(json!({ "providers": [] }))
        }
    }

    #[async_trait]
    impl QueryRuns for UseCaseFake {
        async fn query(&self, query: RunQuery) -> Result<Value, RunError> {
            let _ = query;
            Ok(json!([]))
        }
    }

    fn sample_process_spec() -> ProcessSpec {
        ProcessSpec {
            kind: ProcessKind::Exec,
            command: "echo hi".to_string(),
            language: None,
            args: Vec::new(),
            env: IndexMap::new(),
            cwd: None,
            stdin: None,
            timeout: Some(Milliseconds(1000)),
        }
    }

    #[test]
    fn every_driven_port_is_object_safe_as_dyn() {
        // The composition root injects each driven port as `dyn Port`. Binding one behind each trait
        // object forces the object-safety check at compile time — the negative space is a port that
        // silently became non-`dyn`-able, which would fail to compile here. `Scheduler` is absent by
        // design: its generic method is not vtable-able, so it is used behind a generic bound only.
        let _: Box<dyn ProcessRunner> = Box::new(OkFake);
        let _: Box<dyn HttpClient> = Box::new(OkFake);
        let _: Box<dyn FileSystem> = Box::new(OkFake);
        let _: Box<dyn ObjectStore> = Box::new(OkFake);
        let _: Box<dyn ChatModel> = Box::new(OkFake);
        let _: Box<dyn SecretResolver> = Box::new(OkFake);
        let _: Box<dyn EnvironmentProvider> = Box::new(OkFake);
        let _: Box<dyn RunStore> = Box::new(OkFake);
        let _: Box<dyn EventSink> = Box::new(OkFake);
        let _: Box<dyn SourceLoader> = Box::new(OkFake);
        let _: Box<dyn ReferenceResolver> = Box::new(OkFake);
        let _: Box<dyn SchemaValidator> = Box::new(OkFake);
        let _: Box<dyn Clock> = Box::new(OkFake);
        let boxed_id: Box<dyn IdGenerator> = Box::new(OkFake);

        // Exercise through the trait object, not the concrete type, so the vtable dispatch is real.
        assert_eq!(
            boxed_id.new_run_id().as_str(),
            VALID_UUID_V7,
            "a boxed IdGenerator mints via its vtable"
        );
    }

    #[test]
    fn every_driving_use_case_is_object_safe_as_dyn() {
        // The ten use-case traits back the CLI commands; the binary may hold each as `dyn`. Binding
        // one behind every trait object forces the object-safety check at compile time — the negative
        // space is a use case that silently stopped being `dyn`-able, which would fail to compile.
        let _: Box<dyn RunFlow> = Box::new(UseCaseFake);
        let _: Box<dyn ValidateArtifacts> = Box::new(UseCaseFake);
        let _: Box<dyn LintFlow> = Box::new(UseCaseFake);
        let _: Box<dyn InspectFlow> = Box::new(UseCaseFake);
        let _: Box<dyn ScaffoldFlow> = Box::new(UseCaseFake);
        let _: Box<dyn FormatArtifact> = Box::new(UseCaseFake);
        let _: Box<dyn Discover> = Box::new(UseCaseFake);
        let _: Box<dyn ProvisionEnvironment> = Box::new(UseCaseFake);
        let _: Box<dyn ManageProviders> = Box::new(UseCaseFake);
        let query: Box<dyn QueryRuns> = Box::new(UseCaseFake);

        // Exercise one through its trait object so the vtable dispatch and the `RunError` channel are
        // real, not just a compile check.
        let listed =
            block_on_ready(query.query(RunQuery::List)).expect("a boxed QueryRuns queries");
        assert!(listed.is_array(), "the run listing is a JSON array");
        assert_eq!(listed, json!([]), "the OkFake use case lists no runs");
    }

    #[test]
    fn a_boxed_run_flow_returns_a_terminal_run_record() {
        // The primary use case: drive `RunFlow` through a trait object and confirm the terminal
        // `RunRecord` (a Task-04 model type) routes back with its fields intact.
        let run_flow: Box<dyn RunFlow> = Box::new(UseCaseFake);
        let record = block_on_ready(run_flow.run("deploy", json!({}), RunOptions::default()))
            .expect("the boxed RunFlow runs");
        assert_eq!(
            record.status,
            RunStatus::Ok,
            "the run reached a terminal ok"
        );
        assert_eq!(
            record.flow.as_deref(),
            Some("deploy"),
            "the flow reference rides back on the record"
        );
    }

    #[test]
    fn async_driven_ports_route_results_through_runerror_via_dyn() {
        // Effecting ports are async and returned as `dyn`; drive several through a trait object and
        // confirm the canned `Ok` payloads arrive intact.
        let runner: Box<dyn ProcessRunner> = Box::new(OkFake);
        let out = block_on_ready(runner.run(sample_process_spec())).expect("OkFake runs");
        assert_eq!(
            out.exit_code,
            Some(0),
            "exit code arrives through the boxed port"
        );
        assert_eq!(out.stdout, b"out", "captured stdout arrives intact");

        let http: Box<dyn HttpClient> = Box::new(OkFake);
        let response = block_on_ready(http.send(HttpRequest {
            method: "GET".to_string(),
            url: "https://example.test".to_string(),
            headers: IndexMap::new(),
            query: IndexMap::new(),
            body: None,
            follow_redirects: true,
            retries: 0,
            timeout: None,
        }))
        .expect("OkFake responds");
        assert_eq!(
            response.status, 200,
            "status arrives through the boxed port"
        );
        assert_eq!(response.body, b"body", "the response body arrives intact");

        let fs: Box<dyn FileSystem> = Box::new(OkFake);
        let read = block_on_ready(fs.op(FileOp::Read {
            path: "/tmp/x".to_string(),
            encoding: None,
        }))
        .expect("OkFake reads");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"data".to_vec()
            },
            "the read result matches the op variant"
        );
    }

    #[test]
    fn a_failing_port_propagates_a_typed_runerror() {
        // Negative space: a host failure is a typed `RunError` with its category and context intact,
        // never a panic — the contract 06 fixes for every adapter.
        let runner: Box<dyn ProcessRunner> = Box::new(ErrFake);
        let err = block_on_ready(runner.run(sample_process_spec()))
            .expect_err("ErrFake fails deterministically");
        assert_eq!(
            err.category,
            ErrorCategory::RunFailure,
            "a spawn failure is a run failure, not a panic"
        );
        assert_eq!(
            err.task.as_deref(),
            Some("build"),
            "the failing task's name rides along on the error"
        );
    }

    #[test]
    fn sync_ports_need_no_await_and_still_carry_typed_results() {
        // Clock / IdGenerator / SchemaValidator are the "async only at the effecting boundary" proof:
        // they compute a value with no await point, so they are sync — yet still object-safe and still
        // returning the model/error types.
        let fake = OkFake;
        assert_eq!(
            fake.now().as_str(),
            "2026-07-05T00:00:00Z",
            "the clock reads a fixed instant synchronously"
        );
        assert_eq!(
            fake.now_ms(),
            Milliseconds(42),
            "and a fixed monotonic count"
        );

        let diagnostics = fake
            .validate(&json!({ "kind": "flow" }), ArtifactKind::Flow)
            .expect("validation runs synchronously");
        assert!(
            diagnostics.is_empty(),
            "the valid instance has no diagnostics"
        );
    }

    #[test]
    fn run_store_and_event_sink_accept_the_runtime_model_types() {
        // The driven ports reference Task-04's runtime entities as payloads; exercise the RunStore /
        // EventSink surface to confirm those references type-check and route through `RunError`.
        let store: Box<dyn RunStore> = Box::new(OkFake);
        let id = RunId::new(VALID_UUID_V7).expect("valid id");
        let record = RunRecord {
            id: id.clone(),
            flow: Some("deploy".to_string()),
            status: RunStatus::Ok,
            started_at: Timestamp::new("2026-07-05T00:00:00Z"),
            finished_at: Some(Timestamp::new("2026-07-05T00:00:01Z")),
            ms: Some(Milliseconds(1000)),
            final_state: None,
            results: Vec::new(),
        };
        block_on_ready(store.save(&record)).expect("save accepts a RunRecord");
        assert!(
            block_on_ready(store.get(&id)).expect("get runs").is_none(),
            "the OkFake store holds nothing"
        );

        let sink: Box<dyn EventSink> = Box::new(OkFake);
        let masker = crate::mask::Masker::new();
        let masked = masker.redact_event(&Event::RunStart {
            id,
            flow: "deploy".to_string(),
        });
        block_on_ready(sink.emit(&masked)).expect("the sink accepts a Masked<Event>");
        assert_eq!(
            block_on_ready(store.prune(&Timestamp::new("2026-07-05T00:00:00Z")))
                .expect("prune runs"),
            0,
            "pruning the empty store removes nothing"
        );
    }

    #[test]
    fn scheduler_runs_bounded_and_returns_index_ordered_results() {
        // The Scheduler contract: `count` results, in index order, regardless of `concurrency`. The
        // serial fake makes the ordering observable and asserts the `concurrency >= 1` invariant.
        let scheduler = SerialScheduler;
        let results = block_on_ready(
            scheduler.run_indexed(4, 1, |index| async move { Ok::<u32, RunError>(index * 10) }),
        );
        assert_eq!(results.len(), 4, "one result per index");
        let values: Vec<u32> = results
            .into_iter()
            .map(|r| r.expect("each unit succeeds"))
            .collect();
        assert_eq!(
            values,
            vec![0, 10, 20, 30],
            "results arrive in index order, not completion order"
        );

        // Negative space within the contract: a zero count yields an empty, in-order vector — not a
        // panic and not a stray element.
        let empty =
            block_on_ready(scheduler.run_indexed(0, 2, |i| async move { Ok::<u32, RunError>(i) }));
        assert!(empty.is_empty(), "a zero-width fan-out returns no results");
    }
}
