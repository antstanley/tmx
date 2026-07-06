#![forbid(unsafe_code)]
//! `tmx-schema` — the TMX input data model.
//!
//! Deserialise-only types mirroring the frozen `tmx.schema.json` data-model contract — `Flow`,
//! `Task`, `TaskWith`, `Context`, `Environment`, and the `MatcherName` vocabulary — plus the
//! single source of truth for every runtime **limit** constant (`STATE_SIZE_MAX_BYTES`,
//! `FLOW_DEPTH_MAX`, …). Pure data: no I/O, no async, and no dependency pointing outward, so it
//! sits at the bottom of the workspace dependency graph.
//!
//! Ports: none. This crate declares no port; it is the shared vocabulary that `tmx-core` and the
//! adapters both speak. Task 02 landed the [`limits`] module (every named units-last limit
//! constant) and the closed [`MatcherName`] vocabulary; task 03 adds the deserialise-only
//! Flow/Task/Context/Environment types — a 1:1 mirror of every `$def` in the frozen
//! `tmx.schema.json` input contract, so the whole example corpus loads into typed values with
//! source order preserved.

pub mod context;
pub mod environment;
pub mod flow;
pub mod limits;
pub mod matcher;
pub mod provider;
pub mod task;

/// The TMX spec version this build implements — the version carried in the frozen
/// `tmx.schema.json` `$id` path (`0.2.0` (draft): the `map`/`eval` task types and the `produces`
/// typed-output contract). Surfaced by `tmx version` as the supported spec version. A version
/// identifier string, not a numeric bound, so it lives here beside the data-model vocabulary rather
/// than in [`limits`].
pub const SUPPORTED_SPEC_VERSION: &str = "0.2.0";

pub use context::{Context, EnvMap, Hook, HookUse, Hooks, SecretSource, SecretValue};
pub use environment::{Environment, Resources};
pub use flow::{ContextRef, EnvironmentRef, Flow, InputSpec, TaskEntry, Tasks};
pub use matcher::MatcherName;
pub use provider::{ProviderManifest, ProviderMethodBody, ProviderMethods, ProviderType};
pub use task::{
    AssertWith, Assertion, ChatCompletionWith, ChatMessage, Duration, EvalThreshold, EvalWith,
    ExecWith, FetchWith, FileWith, FlowWith, MapWith, RunWith, Scorer, StoreWith, Task, TaskWith,
};
