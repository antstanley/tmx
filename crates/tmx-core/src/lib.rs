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
//! [`ErrorCategory`]). Task 05 landed the [`ports`] — the driven port traits (every capability the
//! core needs from the outside) and the driving use-case traits (one per CLI command) — with the
//! pure/async boundary fixed at the trait layer: a driven method is async only where it effects the
//! outside world. Adapter and use-case *bodies* arrive in later tasks.

pub mod dispatch;
pub mod error;
pub mod hooks;
pub mod interpolate;
pub mod mask;
pub mod matcher;
pub mod merge;
pub mod model;
pub mod ports;
pub mod preflight;
pub mod resolve;
pub mod runner;
pub mod usecases;

pub use error::{ErrorCategory, RunError};
pub use hooks::{HookKind, HookRunner};
pub use interpolate::evaluate;
pub use mask::{Masked, Masker};
pub use matcher::MatcherEngine;
pub use merge::{AdapterOutput, StateBuilder, normalize_output};
pub use model::{
    BlobWrapper, Diagnostic, EvalCase, EvalSummary, Event, MessageWrapper, Milliseconds, Pipeline,
    PipelineState, ResolvedFlow, RunId, RunRecord, RunStatus, Scope, Scorecard, Severity,
    TaskResult, TaskStatus, Timestamp,
};
pub use preflight::{
    AvailableCapabilities, Capability, CapabilitySet, PreflightPorts, PreflightTarget, Preflighted,
    preflight,
};
pub use resolve::{merged_inputs, resolve_flow};
pub use runner::{PipelineRunner, Ports, ProducesCheck, RunConfig, RunOutcome};
pub use usecases::EngineRunFlow;
