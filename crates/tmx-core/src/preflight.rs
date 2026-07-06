//! Preflight — everything that happens **before the first side effect**: load → `kind` dispatch →
//! reference resolution → directory assembly + desugar → schema validation + limit enforcement →
//! capability check ([03 §Preflight flow](../../../.specs/03-loading-and-preflight.md)).
//!
//! [`preflight`] either passes *wholesale* — returning a [`Preflighted`] carrying the ordered
//! [`ResolvedFlow`] the runner consumes and the [`CapabilitySet`] it can trust — or fails fast with a
//! typed [`RunError`] and **nothing executed**. A directory never runs half-way on a malformed task:
//! validation of every artifact, and the capability check, both complete before [`preflight`]
//! returns, so a failure is discovered here rather than at the first failing task
//! ([03 §Validation](../../../.specs/03-loading-and-preflight.md), §Capability check).
//!
//! It orchestrates over three driven ports — the [`ReferenceResolver`] (resolve a reference/entry to
//! a concrete source), the [`SourceLoader`] (parse it to the JSON model), and the [`SchemaValidator`]
//! (check each artifact) — bundled as [`PreflightPorts`]. The host tells preflight which effecting
//! adapters are *wired and real* via [`AvailableCapabilities`]; a Flow that needs an unwired one is an
//! [`ErrorCategory::Environment`] `missing_capability` naming the port and the task type, up front
//! (03 §Capability check). The crate stays pure: preflight reaches the filesystem only *through* the
//! loader/resolver ports, never directly.

use std::path::Path;

use serde_json::Value;
use tmx_schema::context::{Context, Hook, SecretValue};
use tmx_schema::flow::{ContextRef, EnvironmentRef, Tasks};
use tmx_schema::limits::{
    CONCURRENCY_MAX, FANOUT_WIDTH_MAX, HOOK_TASKS_MAX, JSON_DEPTH_MAX, TASKS_PER_FLOW_MAX,
};
use tmx_schema::task::{Task, TaskWith};
use tmx_schema::{Environment, Flow};

use crate::error::RunError;
use crate::model::{Diagnostic, ResolvedFlow, Severity};
use crate::ports::driven::{ArtifactKind, ReferenceResolver, SchemaValidator, SourceLoader};
use crate::resolve::{desugar_tasks, resolve};

// =============================================================================================
// Capabilities — the effecting driven ports a Flow's task types touch (06 §Executor ports).
// =============================================================================================

/// One effecting capability (driven port) a Flow can require — the vocabulary the capability check
/// gates on (03 §Capability check).
///
/// Only the *effecting* ports are gated: the infrastructure ports the runner always holds
/// (`SourceLoader`/`ReferenceResolver`/`SchemaValidator`/`Clock`/`IdGenerator`/`EventSink`/`RunStore`)
/// are never "missing" and so are not part of this vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// `exec` / `run` — the [`ProcessRunner`](crate::ports::driven::ProcessRunner) port.
    Process,
    /// `fetch` — the [`HttpClient`](crate::ports::driven::HttpClient) port.
    Http,
    /// `file` — the [`FileSystem`](crate::ports::driven::FileSystem) port.
    File,
    /// `store` — the [`ObjectStore`](crate::ports::driven::ObjectStore) port.
    Store,
    /// `chat-completion` / `llmRubric` — the [`ChatModel`](crate::ports::driven::ChatModel) port.
    Chat,
    /// Structured-secret resolution — the [`SecretResolver`](crate::ports::driven::SecretResolver) port.
    Secret,
    /// A provider `environment` block — the
    /// [`EnvironmentProvider`](crate::ports::driven::EnvironmentProvider) port.
    Provider,
}

impl Capability {
    /// Every capability, in declaration order — the closed vocabulary the exhaustive matches pin.
    pub const ALL: [Capability; 7] = [
        Capability::Process,
        Capability::Http,
        Capability::File,
        Capability::Store,
        Capability::Chat,
        Capability::Secret,
        Capability::Provider,
    ];

    /// The stable machine token for this capability (exhaustive `match`, no wildcard).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Capability::Process => "process",
            Capability::Http => "http",
            Capability::File => "file",
            Capability::Store => "store",
            Capability::Chat => "chat",
            Capability::Secret => "secret",
            Capability::Provider => "provider",
        }
    }

    /// The name of the driven **port** this capability is served by — the identifier the
    /// `missing_capability` error names (03 §Capability check).
    #[must_use]
    pub const fn port_name(self) -> &'static str {
        match self {
            Capability::Process => "ProcessRunner",
            Capability::Http => "HttpClient",
            Capability::File => "FileSystem",
            Capability::Store => "ObjectStore",
            Capability::Chat => "ChatModel",
            Capability::Secret => "SecretResolver",
            Capability::Provider => "EnvironmentProvider",
        }
    }
}

/// The set of capabilities a preflighted Flow requires — one entry per distinct [`Capability`], each
/// tagged with the **task type** that first required it (so a `missing_capability` names both the
/// port and the task type, 03 §Capability check).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    required: Vec<(Capability, String)>,
}

impl CapabilitySet {
    /// An empty requirement set — a Flow of only `assert`/`flow` tasks needs no effecting port.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `cap` is required by a task of type `task_type`. The first requirer wins: a later
    /// task of a different type that needs the same port does not overwrite the recorded type, so the
    /// set holds one entry per capability while still naming a concrete requiring task type.
    fn require(&mut self, cap: Capability, task_type: &str) {
        if !self.contains(cap) {
            self.required.push((cap, task_type.to_string()));
        }
    }

    /// Whether `cap` is in the required set.
    #[must_use]
    pub fn contains(&self, cap: Capability) -> bool {
        self.required.iter().any(|(c, _)| *c == cap)
    }

    /// The number of distinct capabilities required.
    #[must_use]
    pub fn len(&self) -> usize {
        self.required.len()
    }

    /// Whether the Flow requires no effecting port.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.required.is_empty()
    }

    /// The required capabilities with the task type that first required each, in requirement order.
    pub fn requirements(&self) -> impl Iterator<Item = (Capability, &str)> {
        self.required.iter().map(|(c, t)| (*c, t.as_str()))
    }
}

/// The effecting capabilities the host has **wired and real** (not absent, not a denying stub) —
/// the "present and real" side of the capability check (03 §Capability check).
///
/// The composition root builds this from its adapter set and hands it to [`preflight`]; a required
/// capability missing from it is a `missing_capability` [`ErrorCategory::Environment`] error.
#[derive(Debug, Clone, Default)]
pub struct AvailableCapabilities {
    present: Vec<Capability>,
}

impl AvailableCapabilities {
    /// No effecting capability is wired — every effecting task type will fail the capability check.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Every effecting capability is wired (the fully-provisioned host).
    #[must_use]
    pub fn all() -> Self {
        Self {
            present: Capability::ALL.to_vec(),
        }
    }

    /// Add `cap` to the wired set (builder form; idempotent).
    #[must_use]
    pub fn with(mut self, cap: Capability) -> Self {
        if !self.has(cap) {
            self.present.push(cap);
        }
        self
    }

    /// Remove `cap` from the wired set (builder form) — models a denying stub for that port.
    #[must_use]
    pub fn without(mut self, cap: Capability) -> Self {
        self.present.retain(|c| *c != cap);
        self
    }

    /// Whether `cap` is wired.
    #[must_use]
    pub fn has(&self, cap: Capability) -> bool {
        self.present.contains(&cap)
    }
}

// =============================================================================================
// Preflight inputs and output.
// =============================================================================================

/// The three driven ports [`preflight`] orchestrates over — a subset of the runner's bundle.
#[derive(Clone, Copy)]
pub struct PreflightPorts<'a> {
    /// Resolves a reference / directory entry to a concrete source (path + format).
    pub reference_resolver: &'a dyn ReferenceResolver,
    /// Parses a resolved source into the JSON model.
    pub source_loader: &'a dyn SourceLoader,
    /// Validates each assembled artifact against the data-model schema.
    pub schema: &'a dyn SchemaValidator,
}

/// What to preflight: a single artifact file, or a directory the caller has already enumerated into
/// its immediate entry references (03 §Directory assembly).
///
/// Directory *enumeration* is I/O and so is the caller's (the CLI's) concern — preflight stays pure
/// and receives the entry references, resolving/loading each through the [`SourceLoader`] port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightTarget {
    /// A single Flow or standalone task file, named by a reference.
    File(String),
    /// A directory: the references of its immediate artifact files (order irrelevant — preflight
    /// imposes natural filename order).
    Directory {
        /// The entry references (paths) discovered in the directory.
        entries: Vec<String>,
    },
}

/// The successful result of [`preflight`]: the runner-ready Flow, its required capabilities, and any
/// non-fatal warnings surfaced during validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Preflighted {
    /// The ordered, desugared, reference-inlined Flow the runner executes.
    pub flow: ResolvedFlow,
    /// The effecting ports the Flow will touch (all confirmed present by the capability check).
    pub capabilities: CapabilitySet,
    /// Non-fatal validation warnings (e.g. the newer-spec compatibility note), emptied by default.
    pub warnings: Vec<Diagnostic>,
}

// =============================================================================================
// The orchestration.
// =============================================================================================

/// One loaded artifact awaiting assembly: its source path, its dispatched class, and its JSON model.
struct LoadedArtifact {
    path: String,
    class: ArtifactClass,
    value: Value,
}

/// Preflight `target` over `ports`, checking required capabilities against `available`.
///
/// Returns a [`Preflighted`] on a wholesale pass, or the first typed [`RunError`] on any failure —
/// a bad reference (`Resolution`), a malformed artifact or breached limit (`Validation`), a duplicate
/// task name (`Resolution`), or a `missing_capability` (`Environment`). Nothing is executed either
/// way: preflight only loads and checks.
///
/// # Errors
///
/// Propagates loader/resolver errors, and raises `Validation` (`too_many_tasks`, `missing_task_name`,
/// `fanout_too_wide`, `json_too_deep`, `concurrency_too_high`, `too_many_hook_tasks`, schema
/// violations), `Resolution` (`duplicate_task_name`), and `Environment` (`missing_capability`).
pub async fn preflight(
    target: &PreflightTarget,
    ports: PreflightPorts<'_>,
    available: &AvailableCapabilities,
) -> Result<Preflighted, RunError> {
    // 1. Load and dispatch every artifact through the loader/resolver ports.
    let artifacts = load_artifacts(target, ports).await?;
    // Preflight is only meaningful over a non-empty target; an empty directory has no Flow to run.
    if artifacts.is_empty() {
        return Err(RunError::validation(
            "empty_target",
            "a preflight target must contain at least one artifact",
        ));
    }

    // 2. Bound each artifact's JSON nesting depth before anything walks it (04 §Limits).
    for artifact in &artifacts {
        if json_depth_exceeds(&artifact.value, JSON_DEPTH_MAX) {
            return Err(RunError::validation(
                "json_too_deep",
                format!(
                    "artifact `{}` nests deeper than the {JSON_DEPTH_MAX}-level JSON depth bound",
                    artifact.path
                ),
            )
            .with_path(artifact.path.clone()));
        }
    }

    // 3. Schema-validate every artifact; a single malformed task aborts here, before any assembly or
    //    execution (03 §Validation). Warnings are collected and returned; errors fail fast.
    let mut warnings = Vec::new();
    for artifact in &artifacts {
        validate_artifact(artifact, ports.schema, &mut warnings)?;
    }

    // 4. Assemble into one Flow, chase every `reference`-form env/context/hook to its inline form
    //    through the ports (03 §Reference resolution), then resolve it (inline env/context, desugar
    //    map/exec into an ordered Vec<Task>). After this pass the Flow is fully inlined, so the
    //    runner-facing `resolve` — and the limit/capability checks below — see a referenced hook's
    //    tasks exactly as they would an inline one.
    let flow = assemble_flow(&artifacts)?;
    let flow = resolve_references(flow, ports, &mut warnings).await?;
    let resolved = resolve(flow)?;

    // 5. Structural + limit checks over the resolved Flow (03 §Validation, 04 §Limits).
    check_structure_and_limits(&resolved)?;

    // 6. Compute the capability set (recursing into map/eval inner tasks, eval scorers, hook bodies,
    //    the provider environment) and verify each required port is wired and real.
    let capabilities = compute_capabilities(&resolved);
    check_capabilities(&capabilities, available)?;

    Ok(Preflighted {
        flow: resolved,
        capabilities,
        warnings,
    })
}

/// Resolve and load every file the `target` names, dispatching each to its [`ArtifactClass`].
async fn load_artifacts(
    target: &PreflightTarget,
    ports: PreflightPorts<'_>,
) -> Result<Vec<LoadedArtifact>, RunError> {
    let references: Vec<String> = match target {
        PreflightTarget::File(reference) => vec![reference.clone()],
        PreflightTarget::Directory { entries } => entries.clone(),
    };
    let mut out = Vec::with_capacity(references.len());
    for reference in &references {
        let resolved = ports.reference_resolver.resolve(reference).await?;
        let value = ports
            .source_loader
            .load(&resolved.path, resolved.kind)
            .await?;
        let class = classify_artifact(&resolved.path, &value)?;
        out.push(LoadedArtifact {
            path: resolved.path,
            class,
            value,
        });
    }
    Ok(out)
}

// =============================================================================================
// Reference resolution — chase `reference`-form env/context/hook bodies to their inline form.
// =============================================================================================

/// Resolve every `reference`-form `environment` / `context` / hook body in `flow` to its inline form,
/// loading + `kind`-dispatching + schema-validating each target through the ports
/// (03 §Reference resolution, §Responsibilities #3).
///
/// After this pass the Flow is **fully inlined**: [`resolve`] (which rejects a `reference`-form
/// env/context) sees only inline forms, and every referenced hook body has become a [`Hook::Tasks`]
/// set — so its tasks count against [`HOOK_TASKS_MAX`] and its ports are pulled into the
/// [`CapabilitySet`], closing the `missing_capability` gap a deferred hook reference would otherwise
/// leave. A dangling reference is the resolver's typed `Resolution` error; a target whose `kind`
/// contradicts the referring field is a `reference_kind_mismatch` `Resolution` error.
async fn resolve_references(
    mut flow: Flow,
    ports: PreflightPorts<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Flow, RunError> {
    // Inline a referenced environment / context first, so their own hook bodies can then be chased.
    let environment_reference = match &flow.environment {
        Some(EnvironmentRef::Reference(reference)) => Some(reference.clone()),
        _ => None,
    };
    if let Some(reference) = environment_reference {
        let environment = load_referenced_environment(&reference, ports, warnings).await?;
        flow.environment = Some(EnvironmentRef::Inline(Box::new(environment)));
    }
    let context_reference = match &flow.context {
        Some(ContextRef::Reference(reference)) => Some(reference.clone()),
        _ => None,
    };
    if let Some(reference) = context_reference {
        let context = load_referenced_context(&reference, ports, warnings).await?;
        flow.context = Some(ContextRef::Inline(Box::new(context)));
    }

    // Chase any referenced hook body inside the (now inline) environment bootstrap and context hooks.
    if let Some(EnvironmentRef::Inline(environment)) = &mut flow.environment
        && let Some(bootstrap) = &mut environment.bootstrap
    {
        resolve_hook_body(bootstrap, ports, warnings).await?;
    }
    if let Some(ContextRef::Inline(context)) = &mut flow.context
        && let Some(hooks) = &mut context.hooks
    {
        for hook in [
            &mut hooks.create,
            &mut hooks.change,
            &mut hooks.destroy,
            &mut hooks.error,
        ]
        .into_iter()
        .flatten()
        {
            resolve_hook_body(hook, ports, warnings).await?;
        }
    }
    Ok(flow)
}

/// Resolve, load, `kind`-dispatch, JSON-depth-bound, and schema-validate one referenced artifact —
/// the shared front half of inlining an env / context / hook target (03 §Reference resolution).
async fn load_referenced_artifact(
    reference: &str,
    ports: PreflightPorts<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<LoadedArtifact, RunError> {
    let resolved = ports.reference_resolver.resolve(reference).await?;
    let value = ports
        .source_loader
        .load(&resolved.path, resolved.kind)
        .await?;
    let class = classify_artifact(&resolved.path, &value)?;
    let artifact = LoadedArtifact {
        path: resolved.path,
        class,
        value,
    };
    if json_depth_exceeds(&artifact.value, JSON_DEPTH_MAX) {
        return Err(RunError::validation(
            "json_too_deep",
            format!(
                "artifact `{}` nests deeper than the {JSON_DEPTH_MAX}-level JSON depth bound",
                artifact.path
            ),
        )
        .with_path(artifact.path.clone()));
    }
    validate_artifact(&artifact, ports.schema, warnings)?;
    Ok(artifact)
}

/// Load a referenced `environment` file and parse it into an [`Environment`], rejecting a target
/// whose explicit `kind` contradicts the referring `environment` field.
async fn load_referenced_environment(
    reference: &str,
    ports: PreflightPorts<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Environment, RunError> {
    let artifact = load_referenced_artifact(reference, ports, warnings).await?;
    ensure_reference_class(&artifact, ArtifactClass::Environment, reference)?;
    parse_artifact(&artifact.value, &artifact.path, "environment_parse_error")
}

/// Load a referenced `context` file and parse it into a [`Context`], rejecting a target whose explicit
/// `kind` contradicts the referring `context` field.
async fn load_referenced_context(
    reference: &str,
    ports: PreflightPorts<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Context, RunError> {
    let artifact = load_referenced_artifact(reference, ports, warnings).await?;
    ensure_reference_class(&artifact, ArtifactClass::Context, reference)?;
    parse_artifact(&artifact.value, &artifact.path, "context_parse_error")
}

/// Resolve a `reference` / `use` hook body in place to its inline [`Hook::Tasks`] form; an inline hook
/// is already resolved and left untouched.
async fn resolve_hook_body(
    hook: &mut Hook,
    ports: PreflightPorts<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), RunError> {
    let reference = match hook {
        Hook::Tasks(_) => return Ok(()),
        Hook::Reference(reference) => reference.clone(),
        Hook::Use(import) => import.use_ref.clone(),
    };
    let tasks = load_referenced_hook_tasks(&reference, ports, warnings).await?;
    *hook = Hook::Tasks(tasks);
    Ok(())
}

/// Load a referenced hook body — a Flow that implements the hook, or a single task file — and return
/// its task collection, so the caller can inline it as a [`Hook::Tasks`] body.
async fn load_referenced_hook_tasks(
    reference: &str,
    ports: PreflightPorts<'_>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Tasks, RunError> {
    let artifact = load_referenced_artifact(reference, ports, warnings).await?;
    match artifact.class {
        ArtifactClass::Flow => Ok(parse_flow(&artifact.value, &artifact.path)?.tasks),
        // A lone task file is a one-task hook body — the same wrapping the standalone-task path uses.
        ArtifactClass::Task => {
            let task = parse_artifact(&artifact.value, &artifact.path, "task_parse_error")?;
            Ok(Tasks::List(vec![task]))
        }
        other => Err(RunError::resolution(
            "reference_kind_mismatch",
            format!(
                "a hook reference must resolve to a flow or task, but `{}` is a {other:?}",
                artifact.path
            ),
        )
        .with_path(artifact.path.clone())),
    }
}

/// Confirm a referenced artifact's dispatched class matches the `expected` role of the referring
/// field. An un-`kind`ed, non-reserved file classifies as [`ArtifactClass::Task`] by default; the
/// referring field is then authoritative, so that fallback is accepted. A file whose *explicit* `kind`
/// (or reserved filename) names a different class is a `reference_kind_mismatch` `Resolution` error.
fn ensure_reference_class(
    artifact: &LoadedArtifact,
    expected: ArtifactClass,
    reference: &str,
) -> Result<(), RunError> {
    if artifact.class == expected {
        return Ok(());
    }
    let has_explicit_kind = artifact.value.get("kind").and_then(Value::as_str).is_some();
    let is_reserved = reserved_stem(&artifact.path).is_some();
    if artifact.class == ArtifactClass::Task && !has_explicit_kind && !is_reserved {
        return Ok(());
    }
    Err(RunError::resolution(
        "reference_kind_mismatch",
        format!(
            "`{reference}` is referenced as {expected:?} but `{}` dispatches to {:?}",
            artifact.path, artifact.class
        ),
    )
    .with_path(artifact.path.clone()))
}

/// Deserialise `value` into `T`, mapping a failure to a typed `code` `Validation` error naming `path`.
fn parse_artifact<T>(value: &Value, path: &str, code: &'static str) -> Result<T, RunError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value.clone()).map_err(|e| {
        RunError::validation(code, format!("could not parse `{path}`: {e}"))
            .with_path(path.to_string())
    })
}

/// Validate one artifact against its schema, folding error-severity diagnostics into a fail-fast
/// `Validation` [`RunError`] and warning/info diagnostics into `warnings`.
///
/// A task artifact has no standalone `kind` in the port vocabulary, so it is validated by wrapping it
/// into a one-task Flow and validating that as a Flow — the exact shape the runner will execute.
fn validate_artifact(
    artifact: &LoadedArtifact,
    schema: &dyn SchemaValidator,
    warnings: &mut Vec<Diagnostic>,
) -> Result<(), RunError> {
    let (instance, kind) = match artifact.class {
        ArtifactClass::Flow => (artifact.value.clone(), ArtifactKind::Flow),
        ArtifactClass::Environment => (artifact.value.clone(), ArtifactKind::Environment),
        ArtifactClass::Context => (artifact.value.clone(), ArtifactKind::Context),
        ArtifactClass::Provider => (artifact.value.clone(), ArtifactKind::Provider),
        // A standalone task validates as a one-task Flow (the port has no `Task` kind).
        ArtifactClass::Task => (wrap_task_as_flow(&artifact.value), ArtifactKind::Flow),
    };
    let diagnostics = schema.validate(&instance, kind)?;
    for diagnostic in diagnostics {
        if matches!(diagnostic.severity, Severity::Error) {
            return Err(RunError::validation(
                "schema_invalid",
                format!(
                    "artifact `{}` failed schema validation: {}",
                    artifact.path, diagnostic.message
                ),
            )
            .with_path(diagnostic.path.unwrap_or_else(|| artifact.path.clone())));
        }
        warnings.push(diagnostic);
    }
    Ok(())
}

/// Wrap a standalone task JSON value into a one-task Flow value `{ "tasks": [ <task> ] }`.
fn wrap_task_as_flow(task: &Value) -> Value {
    serde_json::json!({ "tasks": [task.clone()] })
}

/// Assemble the loaded artifacts into a single [`Flow`] (03 §Directory assembly): a lone Flow file is
/// taken as-is, a lone task file becomes a one-task Flow, and a directory folds its sibling
/// `environment.*` / `context.*` into shared env/context and orders every other artifact as a task by
/// natural filename order.
fn assemble_flow(artifacts: &[LoadedArtifact]) -> Result<Flow, RunError> {
    // A single Flow file resolves directly — no assembly, no synthetic wrapper.
    if artifacts.len() == 1 && artifacts[0].class == ArtifactClass::Flow {
        return parse_flow(&artifacts[0].value, &artifacts[0].path);
    }

    let mut environment: Option<&Value> = None;
    let mut context: Option<&Value> = None;
    let mut tasks: Vec<&LoadedArtifact> = Vec::new();

    for artifact in artifacts {
        match artifact.class {
            ArtifactClass::Environment => {
                if environment.replace(&artifact.value).is_some() {
                    return Err(duplicate_shared("environment", &artifact.path));
                }
            }
            ArtifactClass::Context => {
                if context.replace(&artifact.value).is_some() {
                    return Err(duplicate_shared("context", &artifact.path));
                }
            }
            // A Flow inside a directory is unusual; the spec folds "every other artifact" into a task.
            ArtifactClass::Task | ArtifactClass::Flow | ArtifactClass::Provider => {
                tasks.push(artifact);
            }
        }
    }

    // Natural filename order: byte-wise ASCII, case-sensitive, digit runs compared as integers.
    tasks.sort_by(|a, b| natural_cmp(basename(&a.path), basename(&b.path)));

    let mut flow_object = serde_json::Map::new();
    if let Some(env) = environment {
        flow_object.insert("environment".to_string(), env.clone());
    }
    if let Some(ctx) = context {
        flow_object.insert("context".to_string(), ctx.clone());
    }
    flow_object.insert(
        "tasks".to_string(),
        Value::Array(tasks.iter().map(|t| t.value.clone()).collect()),
    );
    parse_flow(&Value::Object(flow_object), "<assembled directory>")
}

/// Deserialise a Flow value into the [`Flow`] model, mapping a failure to a typed `flow_parse_error`.
fn parse_flow(value: &Value, path: &str) -> Result<Flow, RunError> {
    serde_json::from_value(value.clone()).map_err(|e| {
        RunError::validation(
            "flow_parse_error",
            format!("could not assemble `{path}`: {e}"),
        )
        .with_path(path.to_string())
    })
}

/// A duplicate shared `environment.*` / `context.*` in one directory — the assembly is ambiguous.
fn duplicate_shared(what: &str, path: &str) -> RunError {
    RunError::resolution(
        "duplicate_shared_artifact",
        format!("a directory may hold at most one shared {what}; `{path}` is a second"),
    )
    .with_path(path.to_string())
}

/// Enforce the structural expectations and preflightable limits over the resolved Flow
/// (03 §Validation, 04 §Limits).
fn check_structure_and_limits(flow: &ResolvedFlow) -> Result<(), RunError> {
    // Task-count ceiling.
    if flow.tasks.len() as u64 > u64::from(TASKS_PER_FLOW_MAX) {
        return Err(RunError::validation(
            "too_many_tasks",
            format!(
                "a Flow may hold at most {TASKS_PER_FLOW_MAX} tasks, got {}",
                flow.tasks.len()
            ),
        ));
    }

    // Every task named (non-empty) and unique. A nameless task is a `Validation` `missing_task_name`;
    // a duplicate is a `Resolution` `duplicate_task_name` (03 §Validation).
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for task in &flow.tasks {
        let name = task.name.as_deref().unwrap_or("");
        if name.is_empty() {
            return Err(RunError::validation(
                "missing_task_name",
                "every task must carry a non-empty name before the run",
            ));
        }
        if !seen.insert(name) {
            return Err(RunError::resolution(
                "duplicate_task_name",
                format!("duplicate task name {name:?}"),
            ));
        }
    }

    // Per-task fan-out width (literal collections only) and concurrency ceilings, recursing into
    // map/eval inner tasks.
    for task in &flow.tasks {
        check_task_limits(task)?;
    }

    // Hook-task budget across the context's lifecycle hooks.
    if let Some(context) = &flow.context {
        check_hook_task_budget(context)?;
    }

    Ok(())
}

/// Recursively enforce the fan-out width and concurrency limits on a task and its map/eval inner
/// tasks (04 §Limits).
fn check_task_limits(task: &Task) -> Result<(), RunError> {
    match &task.with {
        TaskWith::Map(map) => {
            check_fanout_width(&map.items, task.name.as_deref())?;
            check_concurrency(map.concurrency, task.name.as_deref())?;
            check_task_limits(&map.task)?;
        }
        TaskWith::Eval(eval) => {
            if let Some(dataset) = &eval.dataset {
                check_fanout_width(dataset, task.name.as_deref())?;
            }
            check_concurrency(eval.concurrency, task.name.as_deref())?;
            if let Some(subject) = &eval.subject {
                check_task_limits(subject)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Reject a literal `items` / `dataset` array wider than [`FANOUT_WIDTH_MAX`]. A `${{ }}` expression
/// resolves at run time, so only a literal array is checkable here (03 §Validation).
fn check_fanout_width(collection: &Value, task: Option<&str>) -> Result<(), RunError> {
    if let Value::Array(items) = collection
        && items.len() as u64 > u64::from(FANOUT_WIDTH_MAX)
    {
        let mut err = RunError::validation(
            "fanout_too_wide",
            format!(
                "a fan-out collection may hold at most {FANOUT_WIDTH_MAX} items, got {}",
                items.len()
            ),
        );
        if let Some(name) = task {
            err = err.with_task(name);
        }
        return Err(err);
    }
    Ok(())
}

/// Reject a task `concurrency` above [`CONCURRENCY_MAX`] (03 §Validation).
fn check_concurrency(concurrency: Option<u32>, task: Option<&str>) -> Result<(), RunError> {
    if let Some(value) = concurrency
        && value > CONCURRENCY_MAX
    {
        let mut err = RunError::validation(
            "concurrency_too_high",
            format!("task concurrency may be at most {CONCURRENCY_MAX}, got {value}"),
        );
        if let Some(name) = task {
            err = err.with_task(name);
        }
        return Err(err);
    }
    Ok(())
}

/// Reject a context whose lifecycle hooks together hold more than [`HOOK_TASKS_MAX`] tasks
/// (04 §Limits).
fn check_hook_task_budget(context: &Context) -> Result<(), RunError> {
    let Some(hooks) = &context.hooks else {
        return Ok(());
    };
    let mut total: u64 = 0;
    for hook in [&hooks.create, &hooks.change, &hooks.destroy, &hooks.error]
        .into_iter()
        .flatten()
    {
        total += hook_task_count(hook)?;
    }
    if total > u64::from(HOOK_TASKS_MAX) {
        return Err(RunError::validation(
            "too_many_hook_tasks",
            format!("a context's hooks may hold at most {HOOK_TASKS_MAX} tasks, got {total}"),
        ));
    }
    Ok(())
}

/// The number of tasks in a hook body. `resolve_references` has already inlined every `reference` /
/// `use` import to a [`Hook::Tasks`] set by the time the budget is checked, so those arms are an
/// unreachable defensive fallback that still contributes nothing rather than panicking.
fn hook_task_count(hook: &Hook) -> Result<u64, RunError> {
    match hook {
        Hook::Reference(_) | Hook::Use(_) => Ok(0),
        Hook::Tasks(tasks) => Ok(desugar_tasks(tasks.clone())?.len() as u64),
    }
}

// =============================================================================================
// Capability computation.
// =============================================================================================

/// Compute the [`CapabilitySet`] a resolved Flow requires, recursing into `map`/`eval` inner tasks,
/// `eval` scorer kinds, lifecycle hook bodies, and the provider environment (03 §Capability check).
fn compute_capabilities(flow: &ResolvedFlow) -> CapabilitySet {
    let mut set = CapabilitySet::new();
    for task in &flow.tasks {
        walk_task_capabilities(task, &mut set);
    }
    if let Some(context) = &flow.context {
        walk_context_capabilities(context, &mut set);
    }
    if let Some(environment) = &flow.environment {
        walk_environment_capabilities(environment, &mut set);
    }
    set
}

/// Accumulate the capabilities one task (and its map/eval descendants) requires.
fn walk_task_capabilities(task: &Task, set: &mut CapabilitySet) {
    match &task.with {
        TaskWith::Exec(_) => set.require(Capability::Process, "exec"),
        TaskWith::Run(_) => set.require(Capability::Process, "run"),
        TaskWith::Fetch(_) => set.require(Capability::Http, "fetch"),
        TaskWith::File(_) => set.require(Capability::File, "file"),
        TaskWith::Store(_) => set.require(Capability::Store, "store"),
        TaskWith::ChatCompletion(_) => set.require(Capability::Chat, "chat-completion"),
        TaskWith::Assert(_) => {}
        TaskWith::Map(map) => walk_task_capabilities(&map.task, set),
        TaskWith::Eval(eval) => {
            if let Some(subject) = &eval.subject {
                walk_task_capabilities(subject, set);
            }
            for scorer in &eval.scorers {
                match scorer.scorer_type.as_deref() {
                    Some("llmRubric") => set.require(Capability::Chat, "eval.llmRubric"),
                    Some("exec") => set.require(Capability::Process, "eval.exec"),
                    Some("run") => set.require(Capability::Process, "eval.run"),
                    _ => {}
                }
            }
        }
        // A `flow` task's inner needs are checked when that sub-flow is itself preflighted.
        TaskWith::Flow(_) => {}
    }
}

/// Accumulate the capabilities a context requires: structured secrets need the `SecretResolver`, and
/// inline hook bodies contribute the ports their tasks touch.
fn walk_context_capabilities(context: &Context, set: &mut CapabilitySet) {
    if let Some(secrets) = &context.secrets
        && secrets
            .values()
            .any(|value| matches!(value, SecretValue::Source(_)))
    {
        set.require(Capability::Secret, "context.secrets");
    }
    if let Some(hooks) = &context.hooks {
        for hook in [&hooks.create, &hooks.change, &hooks.destroy, &hooks.error]
            .into_iter()
            .flatten()
        {
            walk_hook_capabilities(hook, set);
        }
    }
}

/// Accumulate the capabilities a hook body's tasks require. `resolve_references` has already inlined
/// every `reference` / `use` import to a [`Hook::Tasks`] set upstream, so a non-inline arm here is an
/// unreachable defensive fallback (it contributes nothing rather than escaping the capability check).
fn walk_hook_capabilities(hook: &Hook, set: &mut CapabilitySet) {
    if let Hook::Tasks(tasks) = hook
        && let Ok(desugared) = desugar_tasks(tasks.clone())
    {
        for task in &desugared {
            walk_task_capabilities(task, set);
        }
    }
}

/// Accumulate the capabilities an environment requires: a declared provider needs the
/// `EnvironmentProvider`, and inline `bootstrap` tasks contribute the ports they touch.
fn walk_environment_capabilities(environment: &Environment, set: &mut CapabilitySet) {
    if environment.provider.is_some() {
        set.require(Capability::Provider, "environment");
    }
    if let Some(bootstrap) = &environment.bootstrap {
        walk_hook_capabilities(bootstrap, set);
    }
}

/// Verify every required capability is wired and real; the first that is not is a `missing_capability`
/// [`ErrorCategory::Environment`] error naming the port and the requiring task type (03 §Capability
/// check).
fn check_capabilities(
    required: &CapabilitySet,
    available: &AvailableCapabilities,
) -> Result<(), RunError> {
    for (capability, task_type) in required.requirements() {
        if !available.has(capability) {
            return Err(RunError::new(
                crate::error::ErrorCategory::Environment,
                "missing_capability",
                format!(
                    "no {} adapter is wired for a `{task_type}` task",
                    capability.port_name()
                ),
            )
            .with_task(task_type.to_string()));
        }
    }
    Ok(())
}

// =============================================================================================
// Artifact classification, natural ordering, and JSON depth — pure helpers.
// =============================================================================================

/// The class a source document dispatches to — the `kind` column of the 03 dispatch table.
///
/// The core-side mirror of the loader's dispatch: it selects the schema target for validation and the
/// role (shared env/context vs task) in directory assembly. It agrees with the loader adapter's
/// classification by construction — both realise the same 03 §Source loading table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactClass {
    Flow,
    Environment,
    Context,
    Task,
    Provider,
}

/// Classify the loaded `value` at `path` (03 §Source loading and `kind` dispatch): an explicit `kind`
/// wins; then the reserved `environment.*` / `context.*` / `flow.*` filename convention; then "a
/// top-level document with `tasks` is a Flow"; then a task. An out-of-vocabulary explicit `kind` is a
/// typed `unknown_artifact_kind` `Resolution` error.
fn classify_artifact(path: &str, value: &Value) -> Result<ArtifactClass, RunError> {
    if let Some(kind) = value.get("kind").and_then(Value::as_str) {
        return match kind {
            "flow" => Ok(ArtifactClass::Flow),
            "environment" => Ok(ArtifactClass::Environment),
            "context" => Ok(ArtifactClass::Context),
            "task" => Ok(ArtifactClass::Task),
            "provider" => Ok(ArtifactClass::Provider),
            other => Err(RunError::resolution(
                "unknown_artifact_kind",
                format!("unknown artifact kind `{other}` in `{path}`"),
            )
            .with_path(path.to_string())),
        };
    }
    match reserved_stem(path) {
        Some("environment") => return Ok(ArtifactClass::Environment),
        Some("context") => return Ok(ArtifactClass::Context),
        Some("flow") => return Ok(ArtifactClass::Flow),
        _ => {}
    }
    if value.get("tasks").is_some() {
        return Ok(ArtifactClass::Flow);
    }
    Ok(ArtifactClass::Task)
}

/// The reserved filename stem of `path` — the segment before the first `.` of the basename, when it
/// is exactly one of the reserved names (`environment` / `context` / `flow`); otherwise `None`.
fn reserved_stem(path: &str) -> Option<&str> {
    let name = basename(path);
    let stem = name.split_once('.').map_or(name, |(head, _)| head);
    match stem {
        "environment" | "context" | "flow" => Some(stem),
        _ => None,
    }
}

/// The final path component of `path` (its basename), or the whole string when it has no separator.
fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// Compare two filenames in **natural** order (03 §Directory assembly): byte-wise ASCII,
/// case-sensitive and locale-independent, except that maximal runs of ASCII digits compare as
/// unsigned integers — so `task-2` precedes `task-10` and `build` precedes `deploy`, identically on
/// every host.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.bytes().peekable(), b.bytes().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    // Compare two maximal digit runs as unsigned integers, ignoring leading zeros;
                    // on a tie the longer (more leading zeros) run is the greater, keeping a total order.
                    let da = take_digits(&mut ai);
                    let db = take_digits(&mut bi);
                    let na = da.trim_start_matches('0');
                    let nb = db.trim_start_matches('0');
                    let by_len = na.len().cmp(&nb.len());
                    let numeric = if by_len == Ordering::Equal {
                        na.cmp(nb)
                    } else {
                        by_len
                    };
                    let ordering = if numeric == Ordering::Equal {
                        da.len().cmp(&db.len())
                    } else {
                        numeric
                    };
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                } else {
                    let ordering = ca.cmp(&cb);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

/// Consume and return the maximal leading run of ASCII digits from `iter`.
fn take_digits(iter: &mut std::iter::Peekable<std::str::Bytes<'_>>) -> String {
    let mut run = String::new();
    while let Some(&byte) = iter.peek() {
        if byte.is_ascii_digit() {
            run.push(byte as char);
            iter.next();
        } else {
            break;
        }
    }
    run
}

/// Whether any node in `value` nests deeper than `max` levels — bounded, iterative (no unbounded
/// recursion over an adversarially deep document, per Tiger Style).
fn json_depth_exceeds(value: &Value, max: u32) -> bool {
    let mut stack = vec![(value, 1u32)];
    while let Some((node, depth)) = stack.pop() {
        if depth > max {
            return true;
        }
        match node {
            Value::Array(items) => {
                for item in items {
                    stack.push((item, depth + 1));
                }
            }
            Value::Object(map) => {
                for child in map.values() {
                    stack.push((child, depth + 1));
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cmp::Ordering;

    #[test]
    fn natural_cmp_orders_numeric_runs_as_integers_not_bytes() {
        // The load-bearing case: task-2 precedes task-10 (byte order would put "1" < "2" and flip it).
        assert_eq!(natural_cmp("task-2", "task-10"), Ordering::Less);
        assert_eq!(natural_cmp("task-10", "task-2"), Ordering::Greater);
        // Alphabetic runs stay byte-wise; leading zeros tie-break by run length (total order).
        assert_eq!(natural_cmp("build", "deploy"), Ordering::Less);
        assert_eq!(natural_cmp("task-1", "task-1"), Ordering::Equal);
        assert_eq!(natural_cmp("task-02", "task-2"), Ordering::Greater);
    }

    #[test]
    fn natural_cmp_is_case_sensitive_and_locale_independent() {
        // Uppercase 'B' (0x42) sorts before lowercase 'a' (0x61) — pure byte order, no collation.
        assert_eq!(natural_cmp("Build", "apply"), Ordering::Less);
        assert_ne!(natural_cmp("Task", "task"), Ordering::Equal);
    }

    #[test]
    fn json_depth_exceeds_flags_over_deep_but_passes_shallow() {
        // A shallow document is within any positive bound; a deliberately over-deep one trips it.
        // object → array → object → scalar is four levels deep.
        let shallow = json!({ "a": [1, 2, { "b": 3 }] });
        assert!(!json_depth_exceeds(&shallow, JSON_DEPTH_MAX));
        assert!(
            !json_depth_exceeds(&shallow, 4),
            "four levels within a bound of 4"
        );
        assert!(
            json_depth_exceeds(&shallow, 3),
            "four levels breach a bound of 3"
        );

        let mut deep = json!(1);
        for _ in 0..(JSON_DEPTH_MAX + 5) {
            deep = Value::Array(vec![deep]);
        }
        assert!(json_depth_exceeds(&deep, JSON_DEPTH_MAX));
    }

    #[test]
    fn classify_artifact_dispatches_kind_then_filename_then_tasks() {
        assert_eq!(
            classify_artifact("x.yaml", &json!({ "kind": "context" })).expect("kind wins"),
            ArtifactClass::Context
        );
        assert_eq!(
            classify_artifact("dir/environment.toml", &json!({ "platform": "aws" }))
                .expect("filename convention"),
            ArtifactClass::Environment
        );
        assert_eq!(
            classify_artifact("pipeline.yaml", &json!({ "tasks": [] })).expect("tasks ⇒ flow"),
            ArtifactClass::Flow
        );
        assert_eq!(
            classify_artifact("t.jsonc", &json!({ "type": "exec", "with": {} })).expect("task"),
            ArtifactClass::Task
        );
        // Negative space: an out-of-vocabulary explicit kind is a typed resolution error.
        assert!(classify_artifact("x.yaml", &json!({ "kind": "teleport" })).is_err());
    }

    #[test]
    fn capability_set_requires_one_entry_per_port_keeping_first_task_type() {
        let mut set = CapabilitySet::new();
        set.require(Capability::Process, "exec");
        set.require(Capability::Process, "run"); // same port — first requirer kept
        set.require(Capability::Http, "fetch");
        assert_eq!(set.len(), 2, "two distinct ports required");
        assert!(set.contains(Capability::Process) && set.contains(Capability::Http));
        let process = set
            .requirements()
            .find(|(c, _)| *c == Capability::Process)
            .map(|(_, t)| t);
        assert_eq!(
            process,
            Some("exec"),
            "the first requiring task type is kept"
        );
    }

    #[test]
    fn available_capabilities_builder_adds_and_removes() {
        let all = AvailableCapabilities::all();
        assert!(all.has(Capability::Store) && all.has(Capability::Process));
        let missing_store = all.without(Capability::Store);
        assert!(!missing_store.has(Capability::Store), "store removed");
        assert!(missing_store.has(Capability::Process), "process retained");
        let built = AvailableCapabilities::none().with(Capability::Http);
        assert!(built.has(Capability::Http) && !built.has(Capability::File));
    }

    #[test]
    fn capability_tokens_and_ports_are_exhaustive_and_distinct() {
        // Every capability has a distinct machine token and a distinct port name.
        let mut tokens: Vec<&str> = Capability::ALL.iter().map(|c| c.as_str()).collect();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), Capability::ALL.len(), "tokens are distinct");
        let mut ports: Vec<&str> = Capability::ALL.iter().map(|c| c.port_name()).collect();
        ports.sort_unstable();
        ports.dedup();
        assert_eq!(
            ports.len(),
            Capability::ALL.len(),
            "port names are distinct"
        );
    }
}
