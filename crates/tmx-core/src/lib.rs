#![forbid(unsafe_code)]
//! `tmx-core` — the pure hexagonal domain core and the ports.
//!
//! The centre of the hexagon: the runtime model, the typed error model (`RunError` /
//! `ErrorCategory`), the cross-cutting pure services (interpolation, masking, matchers, state
//! merge), the use cases, and the **port traits** — both driving (`RunFlow`, `ValidateArtifacts`,
//! …) and driven (`ProcessRunner`, `HttpClient`, `FileSystem`, `ObjectStore`, `ChatModel`,
//! `SecretResolver`, `EnvironmentProvider`, `RunStore`, `EventSink`, `SourceLoader`,
//! `ReferenceResolver`, `SchemaValidator`, `Clock`, `IdGenerator`, `Scheduler`).
//!
//! Deterministic given its inputs and the ports it is handed: no `tokio`, no `std::fs`, no
//! `std::process`, no `std::time::SystemTime`, no randomness. The pure services are sync; only the
//! driven-port methods are async, and every side effect crosses a port. Depends on `tmx-schema`
//! only — a property the `cargo tree` purity check enforces. The model, errors, ports, and
//! services arrive in tasks 04 onward.
//!
//! Task 04 landed the runtime [`model`] (the entities the engine produces, serialising to the
//! `canonical-types.schema.json` sidecar) and the typed [`error`] model ([`RunError`] /
//! [`ErrorCategory`]).

pub mod error;
pub mod model;

pub use error::{ErrorCategory, RunError};
pub use model::{
    BlobWrapper, Diagnostic, EvalCase, EvalSummary, Event, MessageWrapper, Milliseconds, Pipeline,
    PipelineState, ResolvedFlow, RunId, RunRecord, RunStatus, Scope, Scorecard, Severity,
    TaskResult, TaskStatus, Timestamp,
};
