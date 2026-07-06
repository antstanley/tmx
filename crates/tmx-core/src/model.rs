//! The runtime model — the entities the engine *produces*, plus the two purely-internal runtime
//! structs (`ResolvedFlow`, `Pipeline`, `Scope`).
//!
//! Every schema-backed type here serialises to a `$def` in
//! [`.specs/canonical-types.schema.json`](../../../.specs/canonical-types.schema.json)
//! ([`.specs/01-domain-model.md` §Runtime entities](../../../.specs/01-domain-model.md)). The
//! contract is one-directional: the engine emits these, so every schema-backed type is `Serialize`.
//! Only the value-shaped types that can be *seeded from disk* (`--state-in`, a run listing) are also
//! `Deserialize`; the composite emit-only records (`TaskResult`, `Event`, `RunRecord`, `Diagnostic`)
//! carry a `&'static str` code or an embedded [`RunError`] and are `Serialize`-only, so reading them
//! back is the `RunStore`'s concern (task 27), not this module's.
//!
//! Fixed-width integers cross the serialisation boundary throughout — durations are
//! [`Milliseconds`] (`u64`), counts are `u32` — never `usize`, per
//! [`limits`](tmx_schema::limits)'s convention.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tmx_schema::flow::InputSpec;
use tmx_schema::task::Task;
use tmx_schema::{Context, Environment};

use crate::error::RunError;

// ---------------------------------------------------------------------------------------------
// RunId — structural format constants.
//
// These describe the fixed layout of a UUIDv7 string (RFC 9562, rendered lowercase-hyphenated);
// they are format structure, not tunable *engine* limits, so they live here rather than in
// `tmx-schema::limits` (which is reserved for bounded engine dimensions — state size, depth, fan-out
// counts). They are still named units-last constants, never inline magic numbers.
// ---------------------------------------------------------------------------------------------

/// Length of a hyphenated UUID string, in ASCII characters (`8-4-4-4-12` + four hyphens).
const RUN_ID_LEN_CHARS: usize = 36;
/// The four indices at which a hyphen must appear in a hyphenated UUID.
const RUN_ID_HYPHEN_INDICES: [usize; 4] = [8, 13, 18, 23];
/// The index of the version nibble — must be `7` for a UUIDv7.
const RUN_ID_VERSION_INDEX: usize = 14;
/// The index of the variant nibble — must be one of `8`, `9`, `a`, `b` (the RFC 4122/9562 variant).
const RUN_ID_VARIANT_INDEX: usize = 19;
/// The required version digit for a UUIDv7.
const RUN_ID_VERSION_DIGIT: u8 = b'7';

/// Validate that `s` is a lowercase-hyphenated UUIDv7, matching the `RunId` schema pattern
/// `^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`.
///
/// Returns a [`RunError::validation`] naming the first violation, so a malformed id is an
/// observable typed error rather than a silently-accepted string.
fn validate_run_id(s: &str) -> Result<(), RunError> {
    if !s.is_ascii() || s.len() != RUN_ID_LEN_CHARS {
        return Err(RunError::validation(
            "invalid_run_id",
            format!("a RunId must be a {RUN_ID_LEN_CHARS}-character ASCII UUID, got {s:?}"),
        ));
    }
    for (i, &byte) in s.as_bytes().iter().enumerate() {
        let ok = if RUN_ID_HYPHEN_INDICES.contains(&i) {
            byte == b'-'
        } else if i == RUN_ID_VERSION_INDEX {
            byte == RUN_ID_VERSION_DIGIT
        } else if i == RUN_ID_VARIANT_INDEX {
            matches!(byte, b'8' | b'9' | b'a' | b'b')
        } else {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        };
        if !ok {
            return Err(RunError::validation(
                "invalid_run_id",
                format!("RunId {s:?} violates the UUIDv7 pattern at index {i}"),
            ));
        }
    }
    Ok(())
}

/// A UUIDv7 identifying a single Pipeline run — the `RunId` `$def`.
///
/// A newtype over the lowercase-hyphenated string form (01 §ID scheme). UUIDv7 is time-ordered, so a
/// lexical sort of run ids is chronological — the `RunStore` needs no separate timestamp key. The
/// value is produced by the `IdGenerator` port; this type does not generate one (the core stays
/// deterministic and pulls in no `uuid` dependency), it only carries and validates it. Construction
/// (and deserialisation) enforce the schema pattern, so an ill-formed id is unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RunId(String);

impl RunId {
    /// Construct a `RunId`, validating that `s` is a lowercase-hyphenated UUIDv7. A non-conforming
    /// string is a typed [`RunError::validation`] (`code: invalid_run_id`).
    pub fn new(s: impl Into<String>) -> Result<Self, RunError> {
        let s = s.into();
        validate_run_id(&s)?;
        Ok(Self(s))
    }

    /// The id in its canonical lowercase-hyphenated string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RunId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // A run id read from disk must satisfy the same pattern as a generated one; a corrupt
        // listing entry is a deserialisation error, not a silently-accepted value.
        validate_run_id(&s).map_err(|e| serde::de::Error::custom(e.message))?;
        Ok(Self(s))
    }
}

/// A non-negative whole number of milliseconds — the `Milliseconds` `$def`.
///
/// The normalised wall-clock unit for every duration the engine reports. A `u64` (units-last,
/// fixed-width) so it crosses the serialisation boundary as an integer, never a `usize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Milliseconds(pub u64);

/// An RFC 3339 / ISO 8601 UTC instant — the `Timestamp` `$def`.
///
/// A newtype over the string form, produced via the `Clock` port (the core takes no wall clock of
/// its own, keeping it deterministic). Stored as text because the schema's contract is the RFC 3339
/// string; parsing into a calendar type is a boundary concern, not the model's.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// Wrap an RFC 3339 timestamp string produced by the `Clock` port.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The instant in its RFC 3339 string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The terminal-or-in-flight status of a Pipeline — the `RunStatus` `$def`.
///
/// A closed vocabulary: `ok`/`failed` are terminal success/failure and `cancelled`/`timed_out` are
/// terminal via the cancellation contract; `pending`/`running` are in-flight. No catch-all variant,
/// so an out-of-enum status is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Not yet started.
    Pending,
    /// In flight.
    Running,
    /// Ran to completion successfully (terminal).
    Ok,
    /// A task failed and aborted the run (terminal).
    Failed,
    /// Cancelled via SIGINT (terminal).
    Cancelled,
    /// Cancelled by `--timeout` (terminal).
    TimedOut,
}

impl RunStatus {
    /// Every status, in declaration order — exercises the exhaustiveness of [`RunStatus::as_str`].
    pub const ALL: [RunStatus; 6] = [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::Ok,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::TimedOut,
    ];

    /// The stable `snake_case` wire token for this status. The `match` has no wildcard, so a new
    /// variant cannot ship without a token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Ok => "ok",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::TimedOut => "timed_out",
        }
    }

    /// Whether this is a terminal status (`ok`/`failed`/`cancelled`/`timed_out`). The Pipeline state
    /// machine never leaves a terminal status (01 §Pipeline lifecycle).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        match self {
            RunStatus::Pending | RunStatus::Running => false,
            RunStatus::Ok | RunStatus::Failed | RunStatus::Cancelled | RunStatus::TimedOut => true,
        }
    }
}

/// The outcome of a single task — the `TaskStatus` `$def`.
///
/// `ok` = ran and succeeded; `skipped` = the task's `if` evaluated falsy; `error` = the task failed
/// (aborting the Pipeline, or recorded in place under `continueOnError`). Closed, no catch-all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// The task ran and succeeded.
    Ok,
    /// The task's `if` guard evaluated falsy, so it did not run.
    Skipped,
    /// The task failed.
    Error,
}

impl TaskStatus {
    /// Every status, in declaration order — exercises the exhaustiveness of [`TaskStatus::as_str`].
    pub const ALL: [TaskStatus; 3] = [TaskStatus::Ok, TaskStatus::Skipped, TaskStatus::Error];

    /// The stable `snake_case` wire token for this status. Exhaustive `match`, no wildcard.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Ok => "ok",
            TaskStatus::Skipped => "skipped",
            TaskStatus::Error => "error",
        }
    }
}

/// The single JSON object threaded through a run — the `PipelineState` `$def`.
///
/// Each task's output is merged in under its name: `state[name] = output`. The top level is
/// **always a JSON object** — an invariant this newtype enforces on construction and
/// deserialisation, so a non-object state is unrepresentable. Its serialised size is bounded by the
/// state cap (`tmx-schema::limits::STATE_SIZE_MAX_BYTES`), enforced by the runner (task 10/11), not
/// here.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PipelineState(Value);

impl PipelineState {
    /// Wrap `value` as a Pipeline state, requiring it be a JSON object. A non-object value is a
    /// typed [`RunError::validation`] (`code: state_not_object`) — the top-level-object invariant is
    /// checked, never assumed.
    pub fn new(value: Value) -> Result<Self, RunError> {
        if value.is_object() {
            Ok(Self(value))
        } else {
            Err(RunError::validation(
                "state_not_object",
                "the Pipeline state must be a JSON object at the top level",
            ))
        }
    }

    /// An empty Pipeline state — the `{}` a run starts from.
    #[must_use]
    pub fn empty() -> Self {
        Self(Value::Object(serde_json::Map::new()))
    }

    /// The state as a borrowed JSON value (always [`Value::Object`]).
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// The state's top-level object map. Total: the object invariant is upheld by every constructor.
    #[must_use]
    pub fn as_object(&self) -> &serde_json::Map<String, Value> {
        // The invariant holds by construction; `unwrap_or_else` avoids a panic path in release.
        static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> =
            std::sync::OnceLock::new();
        self.0
            .as_object()
            .unwrap_or_else(|| EMPTY.get_or_init(serde_json::Map::new))
    }

    /// Consume the state, yielding the underlying JSON object value.
    #[must_use]
    pub fn into_value(self) -> Value {
        self.0
    }
}

impl<'de> Deserialize<'de> for PipelineState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        // Seeded from disk (`--state-in`): the top-level-object invariant is enforced on the way in,
        // so a scalar or array seed is a deserialisation error rather than a later surprise.
        Self::new(value).map_err(|e| serde::de::Error::custom(e.message))
    }
}

/// How non-JSON UTF-8 text output is normalised into the Pipeline state — the `MessageWrapper`
/// `$def`. An adapter that returns plain text has it wrapped as `{ "message": <text> }` before the
/// merge, so the state stays JSON objects all the way down (01 §Runtime entities).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageWrapper {
    /// The UTF-8 text output.
    pub message: String,
}

/// How non-UTF-8 binary output is normalised into the Pipeline state — the `BlobWrapper` `$def`.
/// The bytes are base64-encoded (and count toward the state cap) as `{ "blob": <base64> }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobWrapper {
    /// The base64-encoded binary output.
    pub blob: String,
}

/// The severity of a [`Diagnostic`] — the `severity` enum of the `Diagnostic` `$def`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// A blocking finding.
    Error,
    /// A non-blocking finding.
    Warning,
    /// An informational finding.
    Info,
}

impl Severity {
    /// The stable lowercase label for this severity — the token a reporter prefixes a finding with.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// A finding from `validate` or `lint` — the `Diagnostic` `$def`.
///
/// Validation diagnostics come from the `SchemaValidator`; lint diagnostics from static
/// reference/`produces`/secret analysis. `code` is a `&'static str` — the diagnostic codes are a
/// closed compile-time set — so this type is `Serialize`-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// The finding's severity.
    pub severity: Severity,
    /// A stable diagnostic code, e.g. `unknown_task_field`, `undeclared_secret`, `produces_mismatch`.
    pub code: &'static str,
    /// A human-readable message.
    pub message: String,
    /// The location of the finding: a file path with a JSON pointer, or an interpolation expression.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Diagnostic {
    /// Construct a diagnostic with `severity`, a stable `code`, and a `message`, no `path`.
    #[must_use]
    pub fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            path: None,
        }
    }

    /// Attach the location of the finding.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// The per-task record produced by the runner — the `TaskResult` `$def`.
///
/// `output` is present when `status` is `ok` (or `error` under `continueOnError`); `error` is
/// present when `status` is `error`. `Serialize`-only: it embeds the emit-only [`RunError`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    /// The task name — its state key unless overridden by `output`.
    pub name: String,
    /// The task's outcome.
    pub status: TaskStatus,
    /// The JSON the task merged into the state (any JSON type after normalisation), when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// The failure that occurred, when `status` is `error`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
    /// When the task started.
    pub started_at: Timestamp,
    /// The task's wall-clock duration.
    pub ms: Milliseconds,
}

/// One dataset case's scored result inside a [`Scorecard`] — the `EvalCase` `$def`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalCase {
    /// The dataset case object that was evaluated (bound as `${{ case }}`), when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case: Option<Value>,
    /// The subject's output for this case (bound as `${{ output }}`), when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// Per-scorer score in `[0, 1]`, keyed by scorer name in source order.
    pub scores: IndexMap<String, f64>,
    /// The case score: the weighted mean of its scorers' scores, in `[0, 1]`.
    pub score: f64,
    /// Whether the case score is at or above `passScore`.
    pub passed: bool,
}

/// Aggregate metrics over all cases in a [`Scorecard`] — the `EvalSummary` `$def`.
///
/// Includes every metric an `evalThreshold` can gate on (`min` and `p90` among them). `count` is a
/// `u32` — a fixed-width count, not a `usize` — so it crosses the boundary as an integer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalSummary {
    /// The mean case score, in `[0, 1]`.
    pub mean: f64,
    /// The weighted mean case score, in `[0, 1]`.
    pub weighted_mean: f64,
    /// The fraction of cases whose score is at or above `passScore`, in `[0, 1]`.
    pub pass_rate: f64,
    /// The minimum case score, when computed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// The median (50th percentile) case score, when computed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p50: Option<f64>,
    /// The 90th-percentile case score, when computed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p90: Option<f64>,
    /// The number of cases scored.
    pub count: u32,
}

/// The value an `eval` task merges into the Pipeline state — the `Scorecard` `$def`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scorecard {
    /// The per-case results.
    pub cases: Vec<EvalCase>,
    /// The aggregate metrics over all cases.
    pub summary: EvalSummary,
    /// Overall pass/fail: true when no threshold is set, or when the threshold metric meets its min.
    pub passed: bool,
}

/// One entry in the canonical event stream — the `Event` `$def`.
///
/// Internally tagged on the `event` field: a variant serialises to `{ "event": "<name>", … }` with
/// its payload fields flat alongside the tag, matching the schema's flat, `additionalProperties:false`
/// shape (08 §Events & reporters). The ndjson reporter emits one event per line; every payload value
/// passes through the Masker before emission. `Serialize`-only: the `task.error` variant embeds a
/// [`RunError`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event")]
pub enum Event {
    /// A run began. Carries the run id and the flow name.
    #[serde(rename = "run.start")]
    RunStart {
        /// The run id.
        id: RunId,
        /// The flow name.
        flow: String,
    },
    /// A run ended. Carries the run id, terminal status, and total duration.
    #[serde(rename = "run.finish")]
    RunFinish {
        /// The run id.
        id: RunId,
        /// The terminal run status.
        status: RunStatus,
        /// The total run duration.
        ms: Milliseconds,
    },
    /// A task began.
    #[serde(rename = "task.start")]
    TaskStart {
        /// The task name.
        name: String,
    },
    /// A task finished. Carries its outcome, duration, and masked output.
    #[serde(rename = "task.finish")]
    TaskFinish {
        /// The task name.
        name: String,
        /// The task's outcome.
        status: TaskStatus,
        /// The task's duration.
        ms: Milliseconds,
        /// The masked task output, when present.
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
    },
    /// A task was skipped because its `if` evaluated falsy.
    #[serde(rename = "task.skip")]
    TaskSkip {
        /// The task name.
        name: String,
        /// Why it was skipped, e.g. `if=false`.
        reason: String,
    },
    /// A task failed. Carries the typed failure.
    #[serde(rename = "task.error")]
    TaskError {
        /// The task name.
        name: String,
        /// The failure that occurred.
        error: RunError,
    },
    /// A `map` fan-out element finished.
    #[serde(rename = "map.item.finish")]
    MapItemFinish {
        /// The map task's name.
        name: String,
        /// The zero-based item index.
        index: u32,
        /// The element's duration.
        ms: Milliseconds,
    },
    /// An `eval` dataset case finished.
    #[serde(rename = "eval.case.finish")]
    EvalCaseFinish {
        /// The eval task's name.
        name: String,
        /// The zero-based case index.
        index: u32,
    },
    /// A lifecycle hook began.
    #[serde(rename = "hook.start")]
    HookStart {
        /// The hook name.
        name: String,
    },
    /// A lifecycle hook finished.
    #[serde(rename = "hook.finish")]
    HookFinish {
        /// The hook name.
        name: String,
        /// The hook's outcome.
        status: TaskStatus,
        /// The hook's duration.
        ms: Milliseconds,
    },
    /// The per-run event log reached `EVENT_LOG_MAX_BYTES`; persistence stopped (streaming continues).
    #[serde(rename = "log.truncated")]
    LogTruncated,
}

/// What the `RunStore` persists per run at `./.tmx/runs/<id>/` — the `RunRecord` `$def`.
///
/// A record, not a journal: a final-state snapshot plus the ndjson event log, no replay/durability
/// semantics (08 §Run store). `Serialize`-only: its `results` embed the emit-only [`TaskResult`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    /// The run id.
    pub id: RunId,
    /// The flow name or source path, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// The run's terminal (or, mid-run, current) status.
    pub status: RunStatus,
    /// When the run started.
    pub started_at: Timestamp,
    /// When the run finished, when terminal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
    /// The total run duration, when finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms: Option<Milliseconds>,
    /// The merged Pipeline state at run end, secrets masked, when captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_state: Option<PipelineState>,
    /// Per-task results, in execution order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<TaskResult>,
}

// ---------------------------------------------------------------------------------------------
// Purely-internal runtime structs — no schema `$def` (01 §Runtime entities marks them "internal").
// ---------------------------------------------------------------------------------------------

/// A [`Flow`](tmx_schema::Flow) after loading and reference resolution.
///
/// The map-form task list has been sorted into key order and its `exec` shorthands desugared, so the
/// runner only ever sees a fully-formed, ordered `Vec<`[`Task`]`>`. Embeds the task-03 input types
/// ([`Environment`], [`Context`], [`InputSpec`], [`Task`]) rather than re-declaring them. Internal:
/// it is not serialised, so it derives no serde traits.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFlow {
    /// The Flow's optional name.
    pub name: Option<String>,
    /// The Flow's optional description.
    pub description: Option<String>,
    /// The Flow's optional version.
    pub version: Option<String>,
    /// The resolved runtime environment, when the Flow declares one.
    pub environment: Option<Environment>,
    /// The resolved context (env, secrets, hooks), when the Flow declares one.
    pub context: Option<Context>,
    /// The declared inputs, keyed by name in source order.
    pub inputs: IndexMap<String, InputSpec>,
    /// The tasks in execution order, shorthands desugared to full `exec` tasks.
    pub tasks: Vec<Task>,
}

/// A run in flight — the mutable runtime counterpart of a [`ResolvedFlow`].
///
/// Internal: threaded through the runner, never serialised directly (the `RunStore` persists a
/// [`RunRecord`] derived from it).
#[derive(Debug, Clone, PartialEq)]
pub struct Pipeline {
    /// The run id.
    pub id: RunId,
    /// The merged state threaded through the tasks.
    pub state: PipelineState,
    /// The run's current status.
    pub status: RunStatus,
    /// The per-task results accumulated so far, in execution order.
    pub results: Vec<TaskResult>,
}

impl Pipeline {
    /// Start a Pipeline for run `id` with an empty state, `pending` status, and no results yet.
    #[must_use]
    pub fn new(id: RunId) -> Self {
        Self {
            id,
            state: PipelineState::empty(),
            status: RunStatus::Pending,
            results: Vec::new(),
        }
    }
}

/// The read-only binding environment an `${{ … }}` expression sees during a run.
///
/// A struct of borrowed references into the run's namespaces (01 §Required read patterns):
/// `inputs`, `env`, `secrets`, `tasks` (the Pipeline state), and the fan-out-scoped `item`/`case`/
/// `output`/`matrix`. Borrows keep it allocation-free and make it structurally impossible for an
/// expression to mutate the run. The `Interpolator` (task 07) reads it; this task defines its shape.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    /// The declared Flow input values (`${{ inputs.NAME }}`).
    pub inputs: &'a Value,
    /// The resolved context env vars (`${{ env.KEY }}`).
    pub env: &'a Value,
    /// The secrets the task requested (`${{ secrets.NAME }}`); an unrequested name is absent.
    pub secrets: &'a Value,
    /// Prior tasks' merged outputs (`${{ tasks.NAME.field }}`) — the Pipeline state.
    pub tasks: &'a Value,
    /// The current `map` element (`${{ item.* }}`, or the `as:` alias), when inside a fan-out.
    pub item: Option<&'a Value>,
    /// The root name the current `map` element binds under — the task's `as:` alias, defaulting to
    /// `item` when `None`. Lets `${{ region.* }}` resolve the element when `as: region` is declared.
    pub item_alias: Option<&'a str>,
    /// The zero-based position of the current `map` element. Threaded so `${{ item.index }}` (or the
    /// alias's `.index`) resolves for scalar and array elements, which cannot carry a synthetic key.
    pub item_index: Option<u32>,
    /// The current `eval` case (`${{ case.* }}`), when inside an eval.
    pub case: Option<&'a Value>,
    /// The subject's output for the current eval case (`${{ output }}`), when inside an eval.
    pub output: Option<&'a Value>,
    /// The current `--matrix` combination (`${{ matrix.KEY }}`).
    pub matrix: &'a Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_UUID_V7: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

    #[test]
    fn run_id_accepts_a_valid_uuid_v7_and_round_trips() {
        let id = RunId::new(VALID_UUID_V7).expect("a valid UUIDv7 is accepted");
        assert_eq!(id.as_str(), VALID_UUID_V7, "the string form is preserved");

        let json = serde_json::to_value(&id).expect("RunId serialises");
        assert_eq!(
            json,
            Value::String(VALID_UUID_V7.to_string()),
            "serialises transparently"
        );
        let back: RunId = serde_json::from_value(json).expect("RunId round-trips");
        assert_eq!(back, id, "deserialisation reconstructs the same id");
    }

    #[test]
    fn run_id_rejects_malformed_strings() {
        // Negative space: each string violates exactly one facet of the UUIDv7 pattern.
        let bad = [
            ("", "empty is not 36 chars"),
            (
                "018f8c7e-9b2a-7def-8123-456789abcde",
                "35 chars — too short",
            ),
            (
                "018F8C7E-9B2A-7DEF-8123-456789ABCDEF",
                "uppercase hex is rejected",
            ),
            (
                "018f8c7e-9b2a-4def-8123-456789abcdef",
                "version nibble 4, not 7",
            ),
            (
                "018f8c7e-9b2a-7def-c123-456789abcdef",
                "variant nibble c is out of {8,9,a,b}",
            ),
            (
                "018f8c7e_9b2a_7def_8123_456789abcdef",
                "underscores where hyphens belong",
            ),
            (
                "018f8c7e-9b2a-7def-8123-456789abcdeg",
                "'g' is not a hex digit",
            ),
        ];
        for (s, why) in bad {
            assert!(RunId::new(s).is_err(), "RunId::new must reject: {why}");
            assert!(
                serde_json::from_str::<RunId>(&format!("{s:?}")).is_err(),
                "deserialisation must reject: {why}"
            );
        }
    }

    #[test]
    fn pipeline_state_enforces_the_object_invariant() {
        let ok = PipelineState::new(serde_json::json!({ "a": 1 })).expect("an object is accepted");
        assert!(ok.as_value().is_object(), "the wrapped value is an object");
        assert_eq!(ok.as_object().len(), 1, "the object has one key");
        assert!(
            PipelineState::empty().as_object().is_empty(),
            "empty state is an empty object"
        );

        // Negative space: a scalar, an array, and null are all rejected — the top-level-object
        // invariant is unrepresentable to violate through the constructor or deserialisation.
        for non_object in [
            serde_json::json!(42),
            serde_json::json!("text"),
            serde_json::json!([1, 2, 3]),
            serde_json::json!(null),
        ] {
            assert!(
                PipelineState::new(non_object.clone()).is_err(),
                "a non-object {non_object} must be rejected by the constructor"
            );
            let as_text = non_object.to_string();
            assert!(
                serde_json::from_str::<PipelineState>(&as_text).is_err(),
                "a non-object {non_object} must be rejected on deserialisation"
            );
        }
    }

    #[test]
    fn run_status_and_task_status_tokens_match_serialisation_exhaustively() {
        // The exhaustive `as_str` match (no wildcard) plus these loops pin every variant's wire
        // token to its serde form. ALL is the full vocabulary, so a new variant that skips a token
        // fails to compile in `as_str` and is caught here if the arrays disagree.
        for status in RunStatus::ALL {
            let serialised = serde_json::to_value(status).expect("RunStatus serialises");
            assert_eq!(
                serialised,
                Value::String(status.as_str().to_string()),
                "RunStatus {status:?} wire form must equal its as_str token"
            );
        }
        assert_eq!(
            RunStatus::TimedOut.as_str(),
            "timed_out",
            "TimedOut is snake_case timed_out"
        );
        assert!(RunStatus::Ok.is_terminal(), "ok is terminal");
        assert!(!RunStatus::Running.is_terminal(), "running is not terminal");

        for status in TaskStatus::ALL {
            let serialised = serde_json::to_value(status).expect("TaskStatus serialises");
            assert_eq!(
                serialised,
                Value::String(status.as_str().to_string()),
                "TaskStatus {status:?} wire form must equal its as_str token"
            );
        }
    }

    #[test]
    fn event_is_internally_tagged_on_the_event_field() {
        let start = Event::RunStart {
            id: RunId::new(VALID_UUID_V7).expect("valid id"),
            flow: "deploy".to_string(),
        };
        let json = serde_json::to_value(&start).expect("Event serialises");
        assert_eq!(
            json["event"], "run.start",
            "the tag lands under the `event` key"
        );
        assert_eq!(
            json["flow"], "deploy",
            "payload fields sit flat beside the tag"
        );

        // A unit variant serialises to just its tag, nothing more — the log.truncated envelope.
        let truncated = serde_json::to_value(Event::LogTruncated).expect("LogTruncated serialises");
        assert_eq!(
            truncated["event"], "log.truncated",
            "unit variant carries only its tag"
        );
        assert_eq!(
            truncated.as_object().map(serde_json::Map::len),
            Some(1),
            "the log.truncated event has exactly the `event` key"
        );
    }
}
