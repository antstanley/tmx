//! The **driving** port traits — one use case per CLI command
//! ([`.specs/07-cli.md` §Command → use case mapping](../../../../.specs/07-cli.md)).
//!
//! The `tmx` binary is a thin shell: each command is a call into one of these traits, which the
//! composition root wires to a concrete use-case implementation (`usecases.rs`, later tasks). The
//! use cases orchestrate the driven ports, so every method here is `async` at the effecting boundary
//! and carries [`macro@async_trait`] for object safety — the CLI may hold a use case as `dyn` or over
//! a generic bound. Every method returns [`RunError`], the same typed error the driven ports speak,
//! so the CLI's category → exit-code mapping has one vocabulary.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::RunError;
use crate::model::{Diagnostic, RunId, RunRecord};
use crate::ports::driven::{ProviderMethod, ProviderOutcome, SourceKind};

/// Options for a [`RunFlow::run`] — the load-bearing engine flags of `tmx run` (07 §`tmx run`).
///
/// A struct (not a long argument list) so the surface stays stable as flags are added. Kept small:
/// the reporter/format selection is the composition root's concern, not the use case's.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunOptions {
    /// The global `map`/`eval` fan-out concurrency cap (`--concurrency`), when overridden.
    pub concurrency: Option<u32>,
    /// The global wall-clock budget in milliseconds (`--timeout`), when set.
    pub timeout_ms: Option<u64>,
    /// Resolve, validate, and print the plan; execute nothing (`--dry-run`).
    pub dry_run: bool,
    /// Keep the ephemeral environment up after the run (`--keep`).
    pub keep_env: bool,
    /// Do not record this run in the `RunStore` (`--no-store`).
    pub no_store: bool,
}

/// Runs a Flow end to end — the `RunFlow` use case (`tmx run`).
#[must_use = "a RunFlow use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait RunFlow: Send + Sync {
    /// Load → resolve → (provision) → run → report → store the Flow at `reference` with `inputs`
    /// (a JSON object of declared input values) under `options`, returning the terminal
    /// [`RunRecord`]. A failure carries the category the CLI maps to an exit code.
    async fn run(
        &self,
        reference: &str,
        inputs: Value,
        options: RunOptions,
    ) -> Result<RunRecord, RunError>;
}

/// Validates artifacts against the schema — the `ValidateArtifacts` use case (`tmx validate`).
#[must_use = "a ValidateArtifacts use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait ValidateArtifacts: Send + Sync {
    /// Validate each path in `paths` (`kind`-dispatched), returning all [`Diagnostic`]s found (empty
    /// when every artifact is valid).
    async fn validate(&self, paths: Vec<String>) -> Result<Vec<Diagnostic>, RunError>;
}

/// Statically lints a Flow — the `LintFlow` use case (`tmx lint`).
#[must_use = "a LintFlow use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait LintFlow: Send + Sync {
    /// Run resolution + `produces`/secret/import static checks over the Flow at `reference`,
    /// returning any [`Diagnostic`]s.
    async fn lint(&self, reference: &str) -> Result<Vec<Diagnostic>, RunError>;
}

/// The projection [`InspectFlow`] returns — the resolved, ordered view of a Flow (`tmx inspect`).
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    /// The resolved environment + context, ordered plan, declared inputs, and secrets-needed, as one
    /// JSON projection (the shape the CLI renders).
    pub view: Value,
}

/// Inspects a Flow's resolved plan — the `InspectFlow` use case (`tmx inspect`,
/// `tmx context show`, `tmx secrets list`).
#[must_use = "an InspectFlow use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait InspectFlow: Send + Sync {
    /// Resolve the Flow at `reference` and return its [`Inspection`] projection.
    async fn inspect(&self, reference: &str) -> Result<Inspection, RunError>;
}

/// The on-disk layout a [`ScaffoldFlow`] emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaffoldLayout {
    /// A single starter Flow file.
    SingleFile,
    /// A folder layout with separate artifacts.
    Folder,
}

/// Scaffolds a starter Flow — the `ScaffoldFlow` use case (`tmx init`).
#[must_use = "a ScaffoldFlow use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait ScaffoldFlow: Send + Sync {
    /// Create a starter Flow named `name` from `template` in `layout`, returning the paths created.
    async fn scaffold(
        &self,
        name: &str,
        template: &str,
        layout: ScaffoldLayout,
    ) -> Result<Vec<String>, RunError>;
}

/// Reformats a source artifact — the `FormatArtifact` use case (`tmx fmt`).
#[must_use = "a FormatArtifact use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait FormatArtifact: Send + Sync {
    /// Load the artifact at `path` and re-emit it, converting to `to` when set (loss-free across the
    /// four formats) or reformatting in place when `None`. Returns the formatted text.
    async fn format(&self, path: &str, to: Option<SourceKind>) -> Result<String, RunError>;
}

/// What a [`Discover`] enumerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoverKind {
    /// The available Flows.
    Flows,
    /// A Flow's tasks.
    Tasks,
    /// A Flow's declared inputs.
    Inputs,
    /// The registered providers.
    Providers,
}

/// Discovers Flows / tasks / inputs / providers — the `Discover` use case (`tmx list`).
#[must_use = "a Discover use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait Discover: Send + Sync {
    /// Enumerate items of `kind`, scoped to the Flow at `reference` when given, as one JSON listing.
    async fn discover(
        &self,
        kind: DiscoverKind,
        reference: Option<&str>,
    ) -> Result<Value, RunError>;
}

/// Provisions an environment via a provider — the `ProvisionEnvironment` use case (`tmx env`).
#[must_use = "a ProvisionEnvironment use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait ProvisionEnvironment: Send + Sync {
    /// Run provider `method` for the Flow/environment at `reference`, returning the
    /// [`ProviderOutcome`]. A failed method is an
    /// [`ErrorCategory::Environment`](crate::error::ErrorCategory::Environment) error.
    async fn provision(
        &self,
        reference: &str,
        method: ProviderMethod,
    ) -> Result<ProviderOutcome, RunError>;
}

/// A registry operation for [`ManageProviders`] (`tmx provider`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOp {
    /// List registered providers.
    List,
    /// Show one provider's manifest by name.
    Show(String),
    /// Register a provider manifest at a path.
    Register(String),
    /// Remove a registered provider by name.
    Remove(String),
    /// Validate a provider manifest at a path.
    Validate(String),
}

/// Manages the provider registry — the `ManageProviders` use case (`tmx provider`).
#[must_use = "a ManageProviders use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait ManageProviders: Send + Sync {
    /// Perform registry operation `op`, returning its JSON result.
    async fn manage(&self, op: ProviderOp) -> Result<Value, RunError>;
}

/// A query for [`QueryRuns`] over the `RunStore` (`tmx runs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunQuery {
    /// List stored runs.
    List,
    /// Show one run's record.
    Show(RunId),
    /// Show one run's final state.
    State(RunId),
    /// Show one run's event log.
    Logs(RunId),
    /// Prune expired runs.
    Prune,
    /// Remove one run by id.
    Remove(RunId),
}

/// Queries stored runs — the `QueryRuns` use case (`tmx runs`).
#[must_use = "a QueryRuns use case is a port; dropping it discards the wired composition"]
#[async_trait]
pub trait QueryRuns: Send + Sync {
    /// Perform run query `query`, returning its JSON result.
    async fn query(&self, query: RunQuery) -> Result<Value, RunError>;
}
