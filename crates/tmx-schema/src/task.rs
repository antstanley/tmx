//! `Task` and its ten `type`-discriminated payloads. Deserialise-only mirror of the `task` `$def`
//! and every `*With` `$def` in [`docs/tmx.schema.json`](../../../docs/tmx.schema.json).
//!
//! A task is a common envelope (`name`/`if`/`secrets`/`context`/…) plus a `type`-tagged payload
//! carried under `with`. That pairing is modelled as an **adjacently tagged** enum
//! ([`TaskWith`], `tag = "type"`, `content = "with"`) flattened into the [`Task`] envelope, so
//! `{ "type": "exec", "with": { … } }` selects [`TaskWith::Exec`] and deserialises the payload as
//! an [`ExecWith`]. A payload that does not match its `type` — a `fetch`-shaped `with` under
//! `type: exec` — fails to deserialise, because each `*With` struct is `deny_unknown_fields` with
//! its schema-required fields, which is the model's negative space.
//!
//! `MapWith` and `EvalWith` embed a [`Task`], so their variants are boxed to keep [`TaskWith`] a
//! fixed, non-recursive size.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::context::EnvMap;
use crate::flow::ContextRef;
use crate::matcher::MatcherName;

/// The `task` `$def`: one step in a Flow. The common envelope fields plus the `type`-selected
/// payload ([`TaskWith`]) flattened in.
///
/// Unknown top-level keys are ignored rather than rejected: `deny_unknown_fields` cannot coexist
/// with `#[serde(flatten)]`, and the schema's `additionalProperties: false` on a task is a loader
/// concern (task 14), not this deserialise-only mirror's. No corpus artifact carries an unknown
/// task key, so nothing is lost.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Optional artifact discriminator; the constant `"task"` for a standalone task file.
    #[serde(default)]
    pub kind: Option<String>,
    /// Identifier for the task; its output is merged into Pipeline state under this name.
    #[serde(default)]
    pub name: Option<String>,
    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,
    /// Skip condition: a JS-subset expression evaluated against the Pipeline state.
    #[serde(default, rename = "if")]
    pub if_condition: Option<String>,
    /// Names of context secrets to unmask for this task.
    #[serde(default)]
    pub secrets: Option<Vec<String>>,
    /// Task-level context overrides — inline object or a reference.
    #[serde(default)]
    pub context: Option<ContextRef>,
    /// How the task-level context combines with the inherited one (`merge` | `replace`).
    #[serde(default)]
    pub context_strategy: Option<String>,
    /// On a merge collision, which side wins (`local` | `inherited`).
    #[serde(default)]
    pub context_precedence: Option<String>,
    /// Override for the Pipeline-state key the output is merged under (defaults to `name`).
    #[serde(default)]
    pub output: Option<String>,
    /// Optional JSON Schema describing the task's output shape; declarative only.
    #[serde(default)]
    pub produces: Option<Value>,
    /// When true, a failing task does not abort the Pipeline.
    #[serde(default)]
    pub continue_on_error: Option<bool>,
    /// The `type`-selected payload, carried under `with`.
    #[serde(flatten)]
    pub with: TaskWith,
}

/// The ten task implementations, discriminated by `type` with the payload under `with`.
///
/// Adjacently tagged (`tag = "type"`, `content = "with"`): the tag and content keys are flattened
/// into the [`Task`] envelope. Every variant carries a payload, so `with` is required for every
/// `type` — mirroring the schema's `required: ["with"]` on each `allOf` branch.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", content = "with", rename_all = "kebab-case")]
pub enum TaskWith {
    /// Run a shell command.
    Exec(ExecWith),
    /// Run a program/script in a named language/interpreter.
    Run(RunWith),
    /// Perform an HTTP/HTTPS request.
    Fetch(FetchWith),
    /// Read or write local files.
    File(FileWith),
    /// Read from or write to S3-compatible object storage.
    Store(StoreWith),
    /// Call an LLM via the ChatCompletions API.
    ChatCompletion(ChatCompletionWith),
    /// Assert values from the Pipeline state.
    Assert(AssertWith),
    /// Bounded fan-out of an inner task over a collection.
    Map(Box<MapWith>),
    /// Evaluate a subject against scorers and emit a scorecard.
    Eval(Box<EvalWith>),
    /// Import another Flow and run it as a single task.
    Flow(FlowWith),
}

/// The `execWith` `$def`: run a shell command. `additionalProperties: false`, `command` required.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecWith {
    /// The command line to execute (or the executable, when `args` is set).
    pub command: String,
    /// Arguments passed to the command; when set, `command` is the executable.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Shell to run the command in, e.g. `bash`, `sh`, `pwsh`.
    #[serde(default)]
    pub shell: Option<String>,
    /// Working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Environment variables for the command, in source order.
    #[serde(default)]
    pub env: Option<EnvMap>,
    /// Per-command timeout.
    #[serde(default)]
    pub timeout: Option<Duration>,
}

/// The `runWith` `$def`: run a program/script in a named interpreter. `additionalProperties: false`.
/// The schema also requires exactly one of `script`/`file` (`oneOf`); that cross-field rule is a
/// validator concern (task 14), so both are optional here.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunWith {
    /// Interpreter/runtime, e.g. `python`, `node`, `bash`; defaults to `bash`.
    #[serde(default)]
    pub language: Option<String>,
    /// Inline source to execute.
    #[serde(default)]
    pub script: Option<String>,
    /// Path to a script file to execute.
    #[serde(default)]
    pub file: Option<String>,
    /// Arguments passed to the interpreter/script.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Environment variables, in source order.
    #[serde(default)]
    pub env: Option<EnvMap>,
    /// Working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Per-run timeout.
    #[serde(default)]
    pub timeout: Option<Duration>,
}

/// The `fetchWith` `$def`: an HTTP/HTTPS request. `additionalProperties: false`, `url` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FetchWith {
    /// Request URL.
    pub url: String,
    /// HTTP method; defaults to `GET`.
    #[serde(default)]
    pub method: Option<String>,
    /// Request headers.
    #[serde(default)]
    pub headers: Option<IndexMap<String, String>>,
    /// Query-string parameters (values are string/number/boolean).
    #[serde(default)]
    pub query: Option<IndexMap<String, Value>>,
    /// Request body; an object/array is serialised per `bodyType`.
    #[serde(default)]
    pub body: Option<Value>,
    /// How the body is serialised: `json`, `form`, `text`, or `binary`.
    #[serde(default)]
    pub body_type: Option<String>,
    /// Per-request timeout.
    #[serde(default)]
    pub timeout: Option<Duration>,
    /// Whether to follow redirects; defaults to true.
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    /// Number of retries on failure; defaults to 0.
    #[serde(default)]
    pub retries: Option<u32>,
}

/// The `fileWith` `$def`: a local filesystem operation. `additionalProperties: false`;
/// `operation` and `path` required.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileWith {
    /// The operation: `read`, `write`, `append`, `delete`, `copy`, `move`, or `exists`.
    pub operation: String,
    /// Target file path.
    pub path: String,
    /// Content to write (write/append).
    #[serde(default)]
    pub content: Option<String>,
    /// Content encoding: `utf-8`, `base64`, or `binary`.
    #[serde(default)]
    pub encoding: Option<String>,
    /// Destination path for copy/move.
    #[serde(default)]
    pub destination: Option<String>,
}

/// The `storeWith` `$def`: an S3-compatible object-store operation. `additionalProperties: false`;
/// `operation` and `bucket` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoreWith {
    /// The operation: `get`, `put`, `delete`, `list`, or `head`.
    pub operation: String,
    /// Target bucket.
    pub bucket: String,
    /// Object key (required for get/put/delete/head; a prefix for list).
    #[serde(default)]
    pub key: Option<String>,
    /// S3-compatible endpoint URL; omit for AWS defaults.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Region.
    #[serde(default)]
    pub region: Option<String>,
    /// Object body for put.
    #[serde(default)]
    pub content: Option<String>,
    /// Content type for put.
    #[serde(default)]
    pub content_type: Option<String>,
    /// Access credentials; an open object whose values typically reference secrets.
    #[serde(default)]
    pub credentials: Option<Value>,
}

/// The `chatCompletionWith` `$def`: an LLM ChatCompletions call. `additionalProperties: true`;
/// `model` and `messages` required. Extra pass-through keys are captured in
/// [`ChatCompletionWith::extra`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatCompletionWith {
    /// The model identifier.
    pub model: String,
    /// The conversation messages (at least one).
    pub messages: Vec<ChatMessage>,
    /// Full URL of the ChatCompletions endpoint.
    #[serde(default)]
    pub api_url: Option<String>,
    /// API key; typically references a context secret.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Sampling temperature.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Maximum tokens to generate.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Nucleus-sampling `top_p`.
    #[serde(default)]
    pub top_p: Option<f64>,
    /// Whether to stream the response; defaults to false.
    #[serde(default)]
    pub stream: Option<bool>,
    /// Tool/function definitions passed through to the API.
    #[serde(default)]
    pub tools: Option<Vec<Value>>,
    /// Structured-output / response-format directive.
    #[serde(default)]
    pub response_format: Option<Value>,
    /// Any additional API pass-through keys (`additionalProperties: true`), in source order.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// The `chatMessage` `$def`: one conversation message. `additionalProperties: false`;
/// `role` and `content` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatMessage {
    /// The role: `system`, `user`, `assistant`, or `tool`.
    pub role: String,
    /// Message content: a string, an array of content parts, or null.
    pub content: Value,
    /// Optional participant name.
    #[serde(default)]
    pub name: Option<String>,
}

/// The `assertWith` `$def`: assert values from the Pipeline state. `additionalProperties: false`;
/// `assertions` required (at least one).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssertWith {
    /// The assertions to check; the task fails if any does not hold.
    pub assertions: Vec<Assertion>,
}

/// The `assertion` `$def`: one `expect(actual).matcher(expected)` check.
/// `additionalProperties: false`; `actual` and `matcher` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assertion {
    /// The value or `${{ … }}` expression under test.
    pub actual: Value,
    /// The Vitest matcher applied.
    pub matcher: MatcherName,
    /// Negate the matcher (Vitest `.not`); defaults to false.
    #[serde(default)]
    pub not: Option<bool>,
    /// Argument(s) passed to the matcher; omitted for unary matchers.
    #[serde(default)]
    pub expected: Option<Value>,
    /// Custom failure message.
    #[serde(default)]
    pub message: Option<String>,
}

/// The `mapWith` `$def`: bounded fan-out of an inner task over a collection.
/// `additionalProperties: false`; `items` and `task` required. Embeds a [`Task`], hence boxed in
/// [`TaskWith::Map`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MapWith {
    /// The collection to iterate: an inline array, or a `${{ … }}` expression resolving to one.
    pub items: Value,
    /// The inner task run once per item.
    pub task: Task,
    /// Alias the current element is bound under inside the inner task; defaults to `item`.
    #[serde(default, rename = "as")]
    pub as_binding: Option<String>,
    /// Maximum items processed at once; defaults to 1.
    #[serde(default)]
    pub concurrency: Option<u32>,
    /// When true, an item's failure is recorded and iteration continues.
    #[serde(default)]
    pub continue_on_error: Option<bool>,
}

/// The `evalWith` `$def`: evaluate a subject against scorers and emit a scorecard.
/// `additionalProperties: false`; `scorers` required. Embeds a [`Task`], hence boxed in
/// [`TaskWith::Eval`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalWith {
    /// The scorers applied to each case's subject output (at least one).
    pub scorers: Vec<Scorer>,
    /// The thing under evaluation, run once per case; its output is the scored value.
    #[serde(default)]
    pub subject: Option<Task>,
    /// Optional dataset of cases: an inline array, or a `${{ … }}`/reference resolving to one.
    #[serde(default)]
    pub dataset: Option<Value>,
    /// Maximum cases evaluated at once; defaults to 1.
    #[serde(default)]
    pub concurrency: Option<u32>,
    /// Optional gating policy that turns scores into a pass/fail.
    #[serde(default)]
    pub threshold: Option<EvalThreshold>,
}

/// The `scorer` `$def`: one grader applied to a subject output, yielding a score in `[0,1]`.
/// `additionalProperties: false`; `name` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scorer {
    /// Identifier for this scorer; appears in the scorecard under `scores`.
    pub name: String,
    /// Scorer kind: `matcher` (default), `llmRubric`, `exec`, or `run`.
    #[serde(default, rename = "type")]
    pub scorer_type: Option<String>,
    /// Value to score; defaults to the subject's output.
    #[serde(default)]
    pub actual: Option<Value>,
    /// Relative weight in the weighted mean of a case's scorers; defaults to 1.
    #[serde(default)]
    pub weight: Option<f64>,
    /// Optional minimum score for this scorer to count as passing (informational).
    #[serde(default)]
    pub threshold: Option<f64>,
    /// For `type: matcher` — the Vitest matcher applied to `actual`.
    #[serde(default)]
    pub matcher: Option<MatcherName>,
    /// For `type: matcher` — argument passed to the matcher.
    #[serde(default)]
    pub expected: Option<Value>,
    /// For `type: matcher` — negate the matcher.
    #[serde(default)]
    pub not: Option<bool>,
    /// For `type: llmRubric` — instruction describing a good output.
    #[serde(default)]
    pub rubric: Option<String>,
    /// For `type: llmRubric` — the judge model.
    #[serde(default)]
    pub model: Option<String>,
    /// For `type: llmRubric` — full ChatCompletions endpoint URL for the judge.
    #[serde(default)]
    pub api_url: Option<String>,
    /// For `type: llmRubric` — API key; typically references a context secret.
    #[serde(default)]
    pub api_key: Option<String>,
    /// For `type: exec`/`run` — configuration matching `execWith`/`runWith`.
    #[serde(default)]
    pub with: Option<Value>,
}

/// The `evalThreshold` `$def`: a gating policy over an eval's aggregate metric.
/// `additionalProperties: false`; `min` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalThreshold {
    /// Minimum acceptable value of the chosen metric; below this the eval task fails.
    pub min: f64,
    /// Aggregate metric the threshold applies to; defaults to `weightedMean`.
    #[serde(default)]
    pub metric: Option<String>,
    /// Per-case score at/above which a case counts as passing; defaults to 0.5.
    #[serde(default)]
    pub pass_score: Option<f64>,
}

/// The `flowWith` `$def`: import another Flow and run it as a single task.
/// `additionalProperties: false`; `use` required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowWith {
    /// Path/name reference to the Flow to import.
    #[serde(rename = "use")]
    pub use_ref: String,
    /// Input variables passed into the imported Flow, keyed by its declared input names.
    #[serde(default)]
    pub inputs: Option<Value>,
}

/// The `duration` `$def`: an integer number of seconds, or a string like `500ms`, `30s`, `5m`,
/// `1h`.
///
/// Untagged: a JSON integer deserialises to [`Duration::Seconds`], a string to [`Duration::Spec`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Duration {
    /// An integer number of seconds.
    Seconds(u64),
    /// A duration string, e.g. `500ms`, `30s`, `5m`, `1h`.
    Spec(String),
}
