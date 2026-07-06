//! The **driven** port traits — every capability the core needs *from* the outside world, and the
//! request/response payloads they exchange.
//!
//! Each port is a trait `tmx-core` owns and one built-in adapter in `tmx-adapters` implements
//! ([`.specs/06-ports-and-adapters.md`](../../../../.specs/06-ports-and-adapters.md)). The core is
//! generic over them or holds them as `dyn Port` at the composition root; nothing here reaches for an
//! I/O or async-runtime crate, so the purity boundary (`scripts/purity.sh`) stays intact.
//!
//! ## The async / sync split — "async only at the effecting boundary"
//!
//! A method is `async` **iff** it effects the outside world (spawns a process, opens a socket, reads
//! a file, writes a stream). The instantaneous / pure ports are deliberately **sync**: [`Clock`] and
//! [`IdGenerator`] read a value with no await point, and [`SchemaValidator`] is pure CPU over an
//! in-memory JSON value — making them `async` would be ceremony, not correctness. This is the
//! discipline [`architecture-principles.md`](../../../../.specs/architecture-principles.md) §3 names:
//! only the effecting boundary is async.
//!
//! ## Object safety — `#[async_trait]` vs native `async fn`
//!
//! The composition root injects most driven ports as `dyn Port`, and a native `async fn` in a trait
//! is not object-safe. Those ports therefore carry [`macro@async_trait`], which rewrites each method
//! to a `Pin<Box<dyn Future + Send>>` return and restores object safety (proven by the `dyn`
//! assertions in this module's tests). The sync ports need no macro — a sync method is object-safe as
//! written. The lone exception is [`Scheduler`]: its method is generic over the unit type and the
//! per-index closure, so it is *inherently* not object-safe (a generic method cannot live in a
//! vtable). It is used behind a generic bound, never as `dyn`, so it keeps a native `async fn` and
//! opts out of the `async_fn_in_trait` lint locally.

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::{ChatMessage, Environment, SecretSource};

use crate::error::RunError;
use crate::model::{Diagnostic, Milliseconds, RunId, RunRecord, Timestamp};

// =============================================================================================
// Executor ports — one per side-effecting task `type` (06 §Executor ports).
// =============================================================================================

/// Whether a [`ProcessSpec`] is a single shell command line (`exec`) or a script in a named language
/// (`run`, default `bash`) — the `exec` vs `run` distinction of 06 §Executor ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessKind {
    /// `exec`: run one shell command line.
    Exec,
    /// `run`: run a script/program in a named language/interpreter (default `bash`).
    Run,
}

/// What to run through the [`ProcessRunner`] — the request payload for `exec` and `run`.
///
/// A sketch of the resolved, interpolated invocation the runner hands the adapter; the adapter
/// enforces `timeout` and bounds captured output by
/// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessSpec {
    /// Single command line (`exec`) or script body/path (`run`).
    pub kind: ProcessKind,
    /// The command line to execute, or the script body/path for `run`.
    pub command: String,
    /// The language/interpreter for `run` (default `bash`); ignored for `exec`.
    pub language: Option<String>,
    /// Extra arguments passed after the command.
    pub args: Vec<String>,
    /// Environment variables the child sees, in source order.
    pub env: IndexMap<String, String>,
    /// Working directory for the child, when set.
    pub cwd: Option<String>,
    /// Data written to the child's stdin, when set.
    pub stdin: Option<String>,
    /// Per-task wall-clock budget the adapter enforces, when set.
    pub timeout: Option<Milliseconds>,
}

/// The captured result of a [`ProcessRunner::run`] — the `ProcessOutput` sketch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// The child's exit code, or `None` when it was terminated by a signal.
    pub exit_code: Option<i32>,
    /// Captured stdout bytes (bounded by `CAPTURED_OUTPUT_MAX_BYTES`).
    pub stdout: Vec<u8>,
    /// Captured stderr bytes (bounded by `CAPTURED_OUTPUT_MAX_BYTES`).
    pub stderr: Vec<u8>,
    /// The wall-clock duration of the invocation.
    pub ms: Milliseconds,
}

/// Runs a command (`exec`) or a script (`run`) as an external process — the `ProcessRunner` port.
#[must_use = "a ProcessRunner is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait ProcessRunner: Send + Sync {
    /// Run `spec` to completion, returning its captured output. A host failure (spawn error,
    /// non-zero-with-`checked`, timeout) is a typed [`RunError`], never a panic.
    async fn run(&self, spec: ProcessSpec) -> Result<ProcessOutput, RunError>;
}

/// An HTTP request the [`HttpClient`] performs (`fetch`) — the request payload sketch.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    /// The HTTP method, e.g. `GET`, `POST`.
    pub method: String,
    /// The request URL.
    pub url: String,
    /// Request headers, in source order.
    pub headers: IndexMap<String, String>,
    /// Query parameters, in source order.
    pub query: IndexMap<String, String>,
    /// The request body bytes, when present.
    pub body: Option<Vec<u8>>,
    /// Whether the adapter follows redirects.
    pub follow_redirects: bool,
    /// The number of bounded retries on a retryable failure.
    pub retries: u32,
    /// The per-request wall-clock budget, when set.
    pub timeout: Option<Milliseconds>,
}

/// An HTTP response — the `HttpResponse` sketch. The `body` is bounded by `CAPTURED_OUTPUT_MAX_BYTES`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// The HTTP status code.
    pub status: u16,
    /// Response headers, in receive order.
    pub headers: IndexMap<String, String>,
    /// The response body bytes.
    pub body: Vec<u8>,
    /// The wall-clock duration of the request.
    pub ms: Milliseconds,
}

/// Performs an HTTP request — the `HttpClient` port (`fetch`).
#[must_use = "an HttpClient is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait HttpClient: Send + Sync {
    /// Send `request` and return the response. A transport/host failure is a typed [`RunError`].
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RunError>;
}

/// One filesystem operation the [`FileSystem`] port performs (`file`) — the `FileOp` sketch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    /// Read a file's bytes, decoding per `encoding` when set.
    Read {
        /// The path to read.
        path: String,
        /// The declared encoding, when set.
        encoding: Option<String>,
    },
    /// Write `contents`, replacing any existing file.
    Write {
        /// The path to write.
        path: String,
        /// The bytes to write.
        contents: Vec<u8>,
    },
    /// Append `contents` to a file, creating it when absent.
    Append {
        /// The path to append to.
        path: String,
        /// The bytes to append.
        contents: Vec<u8>,
    },
    /// Delete a file.
    Delete {
        /// The path to delete.
        path: String,
    },
    /// Copy a file.
    Copy {
        /// The source path.
        from: String,
        /// The destination path.
        to: String,
    },
    /// Move (rename) a file.
    Move {
        /// The source path.
        from: String,
        /// The destination path.
        to: String,
    },
    /// Test whether a path exists.
    Exists {
        /// The path to test.
        path: String,
    },
}

/// The result of a [`FileSystem::op`] — the `FileResult` sketch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileResult {
    /// The bytes read by [`FileOp::Read`].
    Read {
        /// The file contents.
        contents: Vec<u8>,
    },
    /// The existence answer to [`FileOp::Exists`].
    Exists {
        /// Whether the path exists.
        exists: bool,
    },
    /// A mutating op (`write`/`append`/`delete`/`copy`/`move`) that produced no value completed.
    Done,
}

/// Performs a filesystem operation — the `FileSystem` port (`file`).
#[must_use = "a FileSystem is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// Perform `op`. A host failure (missing path, permission denied) is a typed [`RunError`].
    async fn op(&self, op: FileOp) -> Result<FileResult, RunError>;
}

/// One object-store operation the [`ObjectStore`] port performs (`store`) — the `StoreOp` sketch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOp {
    /// Get an object's bytes.
    Get {
        /// The object key.
        key: String,
    },
    /// Put an object.
    Put {
        /// The object key.
        key: String,
        /// The object bytes.
        body: Vec<u8>,
    },
    /// Delete an object.
    Delete {
        /// The object key.
        key: String,
    },
    /// List keys under a prefix.
    List {
        /// The key prefix to list.
        prefix: String,
    },
    /// Head an object (existence + size, no body).
    Head {
        /// The object key.
        key: String,
    },
}

/// The result of an [`ObjectStore::op`] — the `StoreResult` sketch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreResult {
    /// The bytes returned by [`StoreOp::Get`].
    Get {
        /// The object bytes.
        body: Vec<u8>,
    },
    /// The keys returned by [`StoreOp::List`].
    List {
        /// The listed keys.
        keys: Vec<String>,
    },
    /// The metadata returned by [`StoreOp::Head`].
    Head {
        /// Whether the object exists.
        exists: bool,
        /// The object's size in bytes, when known.
        size_bytes: Option<u64>,
    },
    /// A mutating op (`put`/`delete`) that produced no value completed.
    Done,
}

/// Performs an object-store operation — the `ObjectStore` port (`store`).
#[must_use = "an ObjectStore is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Perform `op`. A remote/host failure is a typed [`RunError`].
    async fn op(&self, op: StoreOp) -> Result<StoreResult, RunError>;
}

/// A chat-completion request — the `ChatRequest` sketch for `chat-completion` and the `llmRubric`
/// scorer. Reuses [`tmx_schema::ChatMessage`] so the message shape matches the input model exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// The model identifier.
    pub model: String,
    /// The conversation, in order.
    pub messages: Vec<ChatMessage>,
    /// The sampling temperature, when set.
    pub temperature: Option<f64>,
    /// The maximum tokens to generate, when set.
    pub max_tokens: Option<u32>,
}

/// A chat-completion response — the `ChatResponse` sketch.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    /// The assistant's message content.
    pub content: String,
    /// The model that produced the response.
    pub model: String,
    /// Prompt tokens consumed, when reported.
    pub prompt_tokens: Option<u32>,
    /// Completion tokens produced, when reported.
    pub completion_tokens: Option<u32>,
    /// The wall-clock duration of the call.
    pub ms: Milliseconds,
}

/// Calls a chat-completion model — the `ChatModel` port (`chat-completion`, `llmRubric`).
#[must_use = "a ChatModel is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait ChatModel: Send + Sync {
    /// Complete `request`. A provider/transport failure is a typed [`RunError`].
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, RunError>;
}

// =============================================================================================
// Cross-cutting driven ports — not tied to a single task type (06 §Cross-cutting driven ports).
// =============================================================================================

/// The four source formats the [`SourceLoader`] and [`ReferenceResolver`] dispatch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// YAML.
    Yaml,
    /// JSON.
    Json,
    /// JSON with comments.
    Jsonc,
    /// TOML.
    Toml,
}

/// Parses a source file into the JSON model, dispatching on `kind` — the `SourceLoader` port.
#[must_use = "a SourceLoader is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait SourceLoader: Send + Sync {
    /// Load and parse the file at `path` as `kind` into a JSON [`Value`]. A read or parse failure is
    /// a typed [`RunError`].
    async fn load(&self, path: &str, kind: SourceKind) -> Result<Value, RunError>;
}

/// A reference resolved to a concrete source — the resolution the [`ReferenceResolver`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSource {
    /// The path the reference resolved to.
    pub path: String,
    /// The source format at that path.
    pub kind: SourceKind,
}

/// Resolves a `reference` string to a concrete source — the `ReferenceResolver` port.
#[must_use = "a ReferenceResolver is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait ReferenceResolver: Send + Sync {
    /// Resolve `reference` (a filesystem path in v0) to a [`ResolvedSource`]. A not-found reference
    /// is a typed [`RunError`].
    async fn resolve(&self, reference: &str) -> Result<ResolvedSource, RunError>;
}

/// The artifact class a [`SchemaValidator`] checks an instance against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A Flow.
    Flow,
    /// A standalone Context.
    Context,
    /// A standalone Environment.
    Environment,
    /// A provider manifest.
    Provider,
}

/// Validates artifacts and `produces` schemas against JSON Schema 2020-12 — the `SchemaValidator`
/// port.
///
/// **Sync**: validation is pure CPU over an in-memory [`Value`] with no effecting boundary, so it
/// needs no `async` and is object-safe as written.
#[must_use = "a SchemaValidator is a port handle; dropping it discards a wired capability"]
pub trait SchemaValidator: Send + Sync {
    /// Validate `instance` as an artifact of `kind`, returning any [`Diagnostic`]s (empty when
    /// valid). An internal failure (e.g. an uncompilable schema) is a typed [`RunError`].
    fn validate(&self, instance: &Value, kind: ArtifactKind) -> Result<Vec<Diagnostic>, RunError>;

    /// Validate a task `output` against a `produces` `schema`, returning any [`Diagnostic`]s.
    fn validate_produces(
        &self,
        output: &Value,
        schema: &Value,
    ) -> Result<Vec<Diagnostic>, RunError>;
}

/// Resolves a `secretSource` to its value — the `SecretResolver` port.
///
/// Returns the resolved secret string; the Masker (task 12) then registers it as sensitive. Sources
/// are `env` / `file` / a named provider (06 §Secret resolution).
#[must_use = "a SecretResolver is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait SecretResolver: Send + Sync {
    /// Resolve `source` to a secret value. A missing env var / file / provider key is a typed
    /// [`RunError`].
    async fn resolve(&self, source: &SecretSource) -> Result<String, RunError>;
}

/// A provider lifecycle method the [`EnvironmentProvider`] invokes — the `environment`-block
/// substrate (06 §Environment and provider execution). Distinct from the *context* lifecycle hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderMethod {
    /// One-time setup of the provider substrate.
    Bootstrap,
    /// Bring an ephemeral environment up before a run.
    Deploy,
    /// Tear an ephemeral environment down after a run (best-effort).
    Clean,
    /// Destroy the provider substrate.
    Destroy,
}

impl ProviderMethod {
    /// Every method, in lifecycle order — the closed vocabulary the manifest's `methods` object names.
    pub const ALL: [ProviderMethod; 4] = [
        ProviderMethod::Bootstrap,
        ProviderMethod::Deploy,
        ProviderMethod::Clean,
        ProviderMethod::Destroy,
    ];

    /// The stable manifest key for this method (`bootstrap`/`deploy`/`clean`/`destroy`) — the name a
    /// [`ProviderMethods`](tmx_schema::ProviderMethods) body is looked up under. Exhaustive `match`,
    /// no wildcard, so a new method cannot ship without a key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ProviderMethod::Bootstrap => "bootstrap",
            ProviderMethod::Deploy => "deploy",
            ProviderMethod::Clean => "clean",
            ProviderMethod::Destroy => "destroy",
        }
    }
}

/// The outcome of a provider method — the result the [`EnvironmentProvider`] returns.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderOutcome {
    /// The method that ran.
    pub method: ProviderMethod,
    /// The method's JSON output.
    pub output: Value,
    /// The wall-clock duration of the method.
    pub ms: Milliseconds,
}

/// Materialises the `environment` block via a provider's lifecycle methods
/// (`bootstrap`/`deploy`/`clean`/`destroy`) — the `EnvironmentProvider` port. Its two v0 adapters are
/// `BinaryProvider` (invokes the manifest binary) and `FlowProvider` (runs the method as a Flow).
#[must_use = "an EnvironmentProvider is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait EnvironmentProvider: Send + Sync {
    /// Invoke `method` against `environment`. A failed method is an
    /// [`ErrorCategory::Environment`](crate::error::ErrorCategory::Environment) [`RunError`].
    async fn invoke(
        &self,
        method: ProviderMethod,
        environment: &Environment,
    ) -> Result<ProviderOutcome, RunError>;
}

/// Persists, queries, and prunes runs — the `RunStore` port (`./.tmx/runs/<id>/`).
#[must_use = "a RunStore is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait RunStore: Send + Sync {
    /// Persist (or update) a run's [`RunRecord`] snapshot.
    async fn save(&self, record: &RunRecord) -> Result<(), RunError>;

    /// Append one [`Event`](crate::model::Event) to run `id`'s event log. Reaching
    /// [`EVENT_LOG_MAX_BYTES`](tmx_schema::limits::EVENT_LOG_MAX_BYTES) is not an error — the adapter
    /// stops persisting and emits a `log.truncated` event.
    async fn append_event(&self, id: &RunId, event: &crate::model::Event) -> Result<(), RunError>;

    /// List the ids of stored runs (chronological by UUIDv7 ordering).
    async fn list(&self) -> Result<Vec<RunId>, RunError>;

    /// Load a run's [`RunRecord`], or `None` when no such run is stored.
    async fn get(&self, id: &RunId) -> Result<Option<RunRecord>, RunError>;

    /// Prune runs older than `cutoff`, returning the number removed.
    async fn prune(&self, cutoff: &Timestamp) -> Result<u32, RunError>;

    /// Remove one run by id.
    async fn remove(&self, id: &RunId) -> Result<(), RunError>;
}

/// Receives the domain [`Event`](crate::model::Event) stream — the `EventSink` port (reporters).
///
/// The payload is a [`Masked<Event>`](crate::mask::Masked): a proven-routed event the runner sealed
/// through the run's [`Masker`](crate::mask::Masker) before emission (08 §Masking at the boundary).
/// Accepting the `Masked` typestate — never a raw `Event` — is what makes it *structurally*
/// impossible for a sink to emit an event that skipped the Masker; each sink additionally asserts the
/// payload's non-zero origin as the paired runtime boundary check.
#[must_use = "an EventSink is a port handle; dropping it discards a wired capability"]
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Emit one masked [`Event`](crate::model::Event), routed through the Masker. A write failure is a
    /// typed [`RunError`].
    async fn emit(&self, event: &crate::mask::Masked<crate::model::Event>) -> Result<(), RunError>;
}

/// The wall-clock and duration source — the `Clock` port (the determinism seam).
///
/// **Sync and infallible**: reading the clock has no await point and cannot fail, so it returns a
/// bare value, not a `Result`. Wrapping it in `Result<_, RunError>` would manufacture an error path
/// that can never be taken — dead negative space the repo definition of done forbids. Injected so a
/// run is reproducible under a fixed clock in tests.
#[must_use = "a Clock is a port handle; dropping it discards a wired capability"]
pub trait Clock: Send + Sync {
    /// The current instant as an RFC 3339 [`Timestamp`].
    fn now(&self) -> Timestamp;

    /// A monotonic millisecond counter for measuring durations (deltas are [`Milliseconds`]). The
    /// origin is unspecified; only differences are meaningful.
    fn now_ms(&self) -> Milliseconds;
}

/// Generates run ids — the `IdGenerator` port (UUIDv7, the determinism seam).
///
/// **Sync and infallible**: minting a UUIDv7 has no await point and cannot fail (unlike
/// [`RunId::new`](crate::model::RunId::new), which *validates* an externally-supplied string), so it
/// returns a bare [`RunId`], not a `Result`. Injected so a run's id is fixed in tests.
#[must_use = "an IdGenerator is a port handle; dropping it discards a wired capability"]
pub trait IdGenerator: Send + Sync {
    /// Mint a fresh [`RunId`] (a UUIDv7).
    fn new_run_id(&self) -> RunId;
}

/// Runs bounded, index-ordered concurrent work — the `Scheduler` port (05 §The Scheduler).
///
/// The single seam for *all* concurrency: it runs `make(i)` for `i in 0..count`, at most
/// `concurrency` in flight, and returns results in **index** order (not completion order). The
/// production adapter bounds in-flight work with a semaphore; the test adapter runs serially for
/// deterministic ordering. `concurrency` is capped by
/// [`CONCURRENCY_MAX`](tmx_schema::limits::CONCURRENCY_MAX) and `count` by
/// [`FANOUT_WIDTH_MAX`](tmx_schema::limits::FANOUT_WIDTH_MAX); the adapter asserts `concurrency >= 1`.
///
/// **Not object-safe**: the method is generic over the unit type `T` and the per-index closure, so it
/// cannot live in a vtable — this port is used behind a generic bound, never as `dyn`. It keeps a
/// native `async fn` (no `#[async_trait]` boxing) and opts out of the `async_fn_in_trait` lint: the
/// returned future's `Send`-ness is bounded per adapter through `Fut: Future + Send` below.
#[must_use = "a Scheduler is a port handle; dropping it discards a wired capability"]
pub trait Scheduler: Send + Sync {
    /// Run `make(i)` for `i in 0..count`, at most `concurrency` concurrently, collecting the results
    /// in **index** order. The returned vector always has length `count`.
    #[allow(async_fn_in_trait)]
    async fn run_indexed<T, F, Fut>(
        &self,
        count: u32,
        concurrency: u32,
        make: F,
    ) -> Vec<Result<T, RunError>>
    where
        T: Send,
        F: Fn(u32) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<T, RunError>> + Send;
}
