//! The `TaskDispatcher` seam — the single `type` → port mapping the runner routes each task through.
//!
//! One exhaustive match over the closed [`TaskWith`] vocabulary
//! ([06 §TaskDispatcher](../../../.specs/06-ports-and-adapters.md)): `assert` is evaluated **inline**
//! through the pure [`MatcherEngine`] (no port), the six side-effecting types
//! (`exec`/`run`/`fetch`/`file`/`store`/`chat-completion`) are routed to their driven ports, and a
//! `flow` task is handed back to the runner to recurse into — guarded first by the
//! [`FLOW_DEPTH_MAX`](tmx_schema::limits::FLOW_DEPTH_MAX) bound so a too-deep nest is a typed
//! `flow_depth_exceeded` [`RunError`] *before* any recursion. `map`/`eval` are the fan-out
//! control-flow types (05): the dispatcher classifies them into [`Dispatch::Map`]/[`Dispatch::Eval`]
//! and the runner drives them over the injected [`Scheduler`](crate::ports::driven::Scheduler).
//!
//! The match has **no `_` wildcard**: adding a variant to the closed Task-03 enum forces a
//! non-exhaustive-match compile error here, so the dispatch table can never drift from the vocabulary.
//!
//! Interpolation lives here too: every `${{ … }}` in a task's config, its `assert` operands, and its
//! `if` gate is resolved against the run [`Scope`] via the pure Task-07 [`evaluate`], with a small
//! template splicer on top so a `${{ }}` embedded in surrounding text renders to a string while a
//! lone `${{ expr }}` preserves the expression's JSON type.

use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::limits::FLOW_DEPTH_MAX;
use tmx_schema::task::{
    AssertWith, ChatCompletionWith, Duration, EvalWith, ExecWith, FetchWith, FileWith, FlowWith,
    MapWith, RunWith, StoreWith, Task, TaskWith,
};
use tmx_schema::{EnvMap, MatcherName};

use crate::error::RunError;
use crate::interpolate::evaluate;
use crate::matcher::MatcherEngine;
use crate::merge::AdapterOutput;
use crate::model::{Milliseconds, Scope};
use crate::ports::driven::{
    ChatRequest, FileOp, FileResult, HttpRequest, ProcessKind, ProcessSpec, StoreOp, StoreResult,
};
use crate::runner::Ports;

/// Milliseconds per second — the `duration` seconds-to-ms conversion factor (units-last, not a
/// tunable engine dimension, so it is a local named constant rather than a `tmx-schema::limits` bound).
const MILLISECONDS_PER_SECOND: u64 = 1000;
/// Milliseconds per minute.
const MILLISECONDS_PER_MINUTE: u64 = 60 * MILLISECONDS_PER_SECOND;
/// Milliseconds per hour.
const MILLISECONDS_PER_HOUR: u64 = 60 * MILLISECONDS_PER_MINUTE;

/// What a single [`dispatch_task`] resolved to: a leaf task's normalised-ready output, a `flow` task
/// the runner must recurse into, or a `map`/`eval` fan-out the runner must orchestrate over the
/// injected [`Scheduler`](crate::ports::driven::Scheduler).
///
/// Splitting the control-flow types (`flow`/`map`/`eval`) back out — rather than executing them inside
/// the dispatcher — keeps this unit free of the runner's loop and the concurrency port while still
/// owning the depth guard: a `flow` variant is only returned once `depth + 1 ≤ FLOW_DEPTH_MAX` holds,
/// and the `map`/`eval` fan-out (which needs the scheduler and the run's masker) is handed back for the
/// runner to drive through [`run_map`](crate::fanout::run_map)/[`run_eval`](crate::fanout::run_eval).
#[derive(Debug)]
pub enum Dispatch<'t> {
    /// A leaf task's raw adapter output, ready for normalisation and merge.
    Leaf(AdapterOutput),
    /// A `flow` task the runner recurses into (the depth guard has already passed).
    Flow(&'t FlowWith),
    /// A `map` task the runner fans out over its `items` through the scheduler.
    Map(&'t MapWith),
    /// An `eval` task the runner fans out over its `dataset` through the scheduler.
    Eval(&'t EvalWith),
}

/// Route `task` (named `name`) through the `type` → port seam against `scope`, at recursion `depth`.
///
/// Returns the leaf output, [`Dispatch::Flow`] for a `flow` task within the depth bound, or
/// [`Dispatch::Map`]/[`Dispatch::Eval`] for the fan-out control-flow types (which the runner drives
/// over the injected scheduler). Every failure is a typed [`RunError`]: an adapter error propagates
/// as-is; a failed `assert` is `assertion_failed`; a too-deep `flow` is `flow_depth_exceeded`.
pub async fn dispatch_task<'t>(
    task: &'t Task,
    name: &str,
    scope: &Scope<'_>,
    ports: Ports<'_>,
    ctx_env: &EnvMap,
    depth: u32,
) -> Result<Dispatch<'t>, RunError> {
    match &task.with {
        TaskWith::Exec(ew) => {
            let spec = build_exec_spec(ew, name, scope, ctx_env)?;
            let out = ports.process.run(spec).await?;
            Ok(Dispatch::Leaf(process_output(out, name)?))
        }
        TaskWith::Run(rw) => {
            let spec = build_run_spec(rw, name, scope, ctx_env)?;
            let out = ports.process.run(spec).await?;
            Ok(Dispatch::Leaf(process_output(out, name)?))
        }
        TaskWith::Fetch(fw) => {
            let request = build_fetch_request(fw, scope)?;
            let response = ports.http.send(request).await?;
            Ok(Dispatch::Leaf(bytes_output(response.body)))
        }
        TaskWith::File(fw) => {
            let op = build_file_op(fw, scope)?;
            let result = ports.file.op(op).await?;
            Ok(Dispatch::Leaf(file_output(result)))
        }
        TaskWith::Store(sw) => {
            let op = build_store_op(sw, scope)?;
            let result = ports.store.op(op).await?;
            Ok(Dispatch::Leaf(store_output(result)))
        }
        TaskWith::ChatCompletion(cw) => {
            let request = build_chat_request(cw, scope)?;
            let response = ports.chat.complete(request).await?;
            Ok(Dispatch::Leaf(AdapterOutput::Json(serde_json::json!({
                "content": response.content,
                "model": response.model,
            }))))
        }
        TaskWith::Assert(aw) => {
            run_assert(aw, scope, name)?;
            Ok(Dispatch::Leaf(AdapterOutput::Json(serde_json::json!({
                "passed": true,
                "assertions": aw.assertions.len() as u64,
            }))))
        }
        // `map`/`eval` are the fan-out control-flow types: the dispatcher classifies them (it does not
        // execute them), handing the typed payload back for the runner to drive over the injected
        // `Scheduler` through `run_map`/`run_eval`. The depth guard for a `flow` inner task lives inside
        // those functions, so — unlike `flow` — the payload is returned without a depth check here.
        TaskWith::Map(mw) => Ok(Dispatch::Map(mw)),
        TaskWith::Eval(ew) => Ok(Dispatch::Eval(ew)),
        TaskWith::Flow(fw) => {
            // The depth guard is a typed error first (input-reachable via a deep nest) and an
            // asserted backstop — the same discipline the state-cap merge uses. `depth >=
            // FLOW_DEPTH_MAX` is `depth + 1 > FLOW_DEPTH_MAX`: recursing would step past the bound.
            if depth >= FLOW_DEPTH_MAX {
                return Err(RunError::resolution(
                    "flow_depth_exceeded",
                    format!(
                        "flow task {name:?} at depth {depth} would recurse past the {FLOW_DEPTH_MAX}-level bound"
                    ),
                )
                .with_task(name));
            }
            // Equivalent to the spec's `depth + 1 <= FLOW_DEPTH_MAX` bound.
            assert!(
                depth < FLOW_DEPTH_MAX,
                "flow recursion must stay within FLOW_DEPTH_MAX"
            );
            Ok(Dispatch::Flow(fw))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Interpolation — the `${{ … }}` template layer over the pure Task-07 expression evaluator.
// ---------------------------------------------------------------------------------------------

/// The opening delimiter of a `${{ … }}` interpolation.
const INTERP_OPEN: &str = "${{";
/// The closing delimiter of a `${{ … }}` interpolation.
const INTERP_CLOSE: &str = "}}";

/// Interpolate a template `s` against `scope`.
///
/// A string with no `${{` is returned verbatim. A string that is exactly one `${{ expr }}` spanning
/// the whole (trimmed) value returns the expression's raw JSON [`Value`] — so `${{ inputs.count }}`
/// stays a number. Otherwise every `${{ expr }}` is evaluated, rendered to text, and spliced into the
/// surrounding string.
pub(crate) fn interp_template(s: &str, scope: &Scope<'_>) -> Result<Value, RunError> {
    if !s.contains(INTERP_OPEN) {
        return Ok(Value::String(s.to_string()));
    }
    let trimmed = s.trim();
    if let Some(inner) = trimmed
        .strip_prefix(INTERP_OPEN)
        .and_then(|r| r.strip_suffix(INTERP_CLOSE))
        && !inner.contains(INTERP_OPEN)
    {
        return evaluate(inner.trim(), scope);
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find(INTERP_OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + INTERP_OPEN.len()..];
        let Some(end) = after.find(INTERP_CLOSE) else {
            return Err(RunError::resolution(
                "expr_unterminated",
                format!("unterminated {INTERP_OPEN} in template {s:?}"),
            ));
        };
        let value = evaluate(after[..end].trim(), scope)?;
        out.push_str(&value_to_str(&value));
        rest = &after[end + INTERP_CLOSE.len()..];
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

/// Deep-interpolate every string leaf of `value` (arrays and objects recurse, scalars pass through).
pub(crate) fn interp_value(value: &Value, scope: &Scope<'_>) -> Result<Value, RunError> {
    match value {
        Value::String(s) => interp_template(s, scope),
        Value::Array(items) => items
            .iter()
            .map(|item| interp_value(item, scope))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, val) in map {
                out.insert(key.clone(), interp_value(val, scope)?);
            }
            Ok(Value::Object(out))
        }
        scalar => Ok(scalar.clone()),
    }
}

/// Interpolate `s` and coerce the result to a string (a scalar to its text, a compound to JSON).
fn interp_to_string(s: &str, scope: &Scope<'_>) -> Result<String, RunError> {
    interp_template(s, scope).map(|v| value_to_str(&v))
}

/// Render a JSON [`Value`] to a string for template splicing: a string yields its contents, any
/// other value its compact JSON form.
fn value_to_str(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// JS truthiness over a JSON value — `false`/`null`/`0`/`NaN`/`""` are falsy, everything else truthy.
///
/// The `if`-gate predicate: the runner skips a task whose `if` interpolates to a falsy value.
#[must_use]
pub(crate) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_none_or(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

// ---------------------------------------------------------------------------------------------
// `assert` — inline matcher evaluation, no port.
// ---------------------------------------------------------------------------------------------

/// Evaluate every assertion in `aw` against `scope`; the first that does not hold is a typed
/// `assertion_failed` [`RunError`] naming the task. Operands are interpolated before matching.
fn run_assert(aw: &AssertWith, scope: &Scope<'_>, name: &str) -> Result<(), RunError> {
    for (index, assertion) in aw.assertions.iter().enumerate() {
        let actual = interp_value(&assertion.actual, scope)?;
        let expected = assertion
            .expected
            .as_ref()
            .map(|e| interp_value(e, scope))
            .transpose()?;
        let args = split_args(assertion.matcher, expected.as_ref());
        let not = assertion.not.unwrap_or(false);
        let held = MatcherEngine::evaluate(&actual, assertion.matcher, args.as_deref(), not);
        if !held {
            let message = assertion.message.clone().unwrap_or_else(|| {
                format!(
                    "assertion #{index} ({}{}) did not hold",
                    if not { "not " } else { "" },
                    assertion.matcher.as_str()
                )
            });
            return Err(RunError::run_failure("assertion_failed", message).with_task(name));
        }
    }
    Ok(())
}

/// Split an assertion's interpolated `expected` into the argument slice the [`MatcherEngine`] takes.
///
/// A unary matcher gets `None`. The two multi-argument matchers (`toHaveProperty(path, value)`,
/// `toBeCloseTo(number, precision)`) take an array `expected` as their argument list; every other
/// matcher treats `expected` as a single argument (so a `toEqual([1,2])` compares against the whole
/// array, not two arguments).
pub(crate) fn split_args(matcher: MatcherName, expected: Option<&Value>) -> Option<Vec<Value>> {
    let expected = expected?;
    match matcher {
        MatcherName::ToHaveProperty | MatcherName::ToBeCloseTo => match expected {
            Value::Array(args) => Some(args.clone()),
            single => Some(vec![single.clone()]),
        },
        _ => Some(vec![expected.clone()]),
    }
}

// ---------------------------------------------------------------------------------------------
// Request builders — interpolate the typed `with` payloads into port request structs.
// ---------------------------------------------------------------------------------------------

/// Build a [`ProcessSpec`] for an `exec` task.
pub(crate) fn build_exec_spec(
    ew: &ExecWith,
    name: &str,
    scope: &Scope<'_>,
    ctx_env: &EnvMap,
) -> Result<ProcessSpec, RunError> {
    let _ = name;
    Ok(ProcessSpec {
        kind: ProcessKind::Exec,
        command: interp_to_string(&ew.command, scope)?,
        language: ew.shell.clone(),
        args: interp_args(ew.args.as_deref(), scope)?,
        env: build_env(ctx_env, ew.env.as_ref(), scope)?,
        cwd: ew
            .cwd
            .as_deref()
            .map(|c| interp_to_string(c, scope))
            .transpose()?,
        stdin: None,
        timeout: ew.timeout.as_ref().and_then(duration_to_ms),
    })
}

/// Build a [`ProcessSpec`] for a `run` task (a script in a named interpreter, default `bash`).
pub(crate) fn build_run_spec(
    rw: &RunWith,
    name: &str,
    scope: &Scope<'_>,
    ctx_env: &EnvMap,
) -> Result<ProcessSpec, RunError> {
    let command = match (&rw.script, &rw.file) {
        (Some(script), _) => interp_to_string(script, scope)?,
        (None, Some(file)) => interp_to_string(file, scope)?,
        (None, None) => {
            return Err(RunError::validation(
                "run_missing_source",
                "a run task needs exactly one of `script` or `file`",
            )
            .with_task(name));
        }
    };
    Ok(ProcessSpec {
        kind: ProcessKind::Run,
        command,
        language: Some(rw.language.clone().unwrap_or_else(|| "bash".to_string())),
        args: interp_args(rw.args.as_deref(), scope)?,
        env: build_env(ctx_env, rw.env.as_ref(), scope)?,
        cwd: rw
            .cwd
            .as_deref()
            .map(|c| interp_to_string(c, scope))
            .transpose()?,
        stdin: None,
        timeout: rw.timeout.as_ref().and_then(duration_to_ms),
    })
}

/// Interpolate an optional argument list, defaulting to no arguments.
fn interp_args(args: Option<&[String]>, scope: &Scope<'_>) -> Result<Vec<String>, RunError> {
    args.unwrap_or(&[])
        .iter()
        .map(|a| interp_to_string(a, scope))
        .collect()
}

/// Merge the resolved context env (base) with the task's own `with.env` (overlay), interpolating
/// every value against `scope`; the task's values win on a key collision.
fn build_env(
    ctx_env: &EnvMap,
    task_env: Option<&EnvMap>,
    scope: &Scope<'_>,
) -> Result<IndexMap<String, String>, RunError> {
    let mut out = IndexMap::new();
    for (key, value) in ctx_env {
        out.insert(key.clone(), interp_to_string(value, scope)?);
    }
    if let Some(task_env) = task_env {
        for (key, value) in task_env {
            out.insert(key.clone(), interp_to_string(value, scope)?);
        }
    }
    Ok(out)
}

/// Build an [`HttpRequest`] for a `fetch` task.
fn build_fetch_request(fw: &FetchWith, scope: &Scope<'_>) -> Result<HttpRequest, RunError> {
    let mut headers = IndexMap::new();
    if let Some(source) = &fw.headers {
        for (key, value) in source {
            headers.insert(key.clone(), interp_to_string(value, scope)?);
        }
    }
    let mut query = IndexMap::new();
    if let Some(source) = &fw.query {
        for (key, value) in source {
            query.insert(key.clone(), value_to_str(&interp_value(value, scope)?));
        }
    }
    // Serialise the body per `bodyType` (schema default `json`), and set a matching `Content-Type`
    // unless the task already declared one — so `fetch` honours `bodyType` end-to-end while a
    // caller-supplied header always wins.
    let body = match &fw.body {
        None => None,
        Some(value) => {
            let resolved = interp_value(value, scope)?;
            let body_type = fw.body_type.as_deref().unwrap_or(FETCH_BODY_TYPE_DEFAULT);
            let (bytes, content_type) = serialize_fetch_body(&resolved, body_type);
            if !headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case(CONTENT_TYPE_HEADER))
            {
                headers.insert(CONTENT_TYPE_HEADER.to_string(), content_type.to_string());
            }
            Some(bytes)
        }
    };
    Ok(HttpRequest {
        method: fw
            .method
            .as_deref()
            .map(|m| interp_to_string(m, scope))
            .transpose()?
            .unwrap_or_else(|| "GET".to_string()),
        url: interp_to_string(&fw.url, scope)?,
        headers,
        query,
        body,
        follow_redirects: fw.follow_redirects.unwrap_or(true),
        retries: fw.retries.unwrap_or(0),
        timeout: fw.timeout.as_ref().and_then(duration_to_ms),
    })
}

/// The `bodyType` used when a `fetch` task omits it — the schema default (`json`).
const FETCH_BODY_TYPE_DEFAULT: &str = "json";
/// The header name a serialised `fetch` body sets a default value for.
const CONTENT_TYPE_HEADER: &str = "Content-Type";

/// Serialise a resolved `fetch` body per `body_type`, returning the bytes and the default
/// `Content-Type` for that type. `json` (and any unrecognised type — the schema constrains the input,
/// so this is a defensive fallback, not a silent accept) serialises as compact JSON; `form` as
/// `application/x-www-form-urlencoded`; `text`/`binary` as the raw string bytes.
fn serialize_fetch_body(value: &Value, body_type: &str) -> (Vec<u8>, &'static str) {
    match body_type {
        "form" => (
            form_urlencode(value).into_bytes(),
            "application/x-www-form-urlencoded",
        ),
        "text" => (
            value_to_str(value).into_bytes(),
            "text/plain; charset=utf-8",
        ),
        "binary" => (value_to_str(value).into_bytes(), "application/octet-stream"),
        _ => (
            // `to_vec` on a `Value` does not fail in practice; the fallback keeps this panic-free
            // (no `unwrap`) if it ever did.
            serde_json::to_vec(value).unwrap_or_else(|_| value_to_str(value).into_bytes()),
            "application/json",
        ),
    }
}

/// Encode a `form` body as `application/x-www-form-urlencoded`. An object becomes `k=v&k2=v2` with
/// each key and value percent-encoded; any non-object value falls back to a single percent-encoded
/// field so a mis-shaped body is still transmitted rather than dropped.
fn form_urlencode(value: &Value) -> String {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, val)| {
                format!(
                    "{}={}",
                    percent_encode(key),
                    percent_encode(&value_to_str(val))
                )
            })
            .collect::<Vec<_>>()
            .join("&"),
        other => percent_encode(&value_to_str(other)),
    }
}

/// Percent-encode `input` for `application/x-www-form-urlencoded`: unreserved characters
/// (`A–Z a–z 0–9 - _ . ~`) pass through, a space becomes `+`, and every other byte becomes `%XX`.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Build a [`FileOp`] for a `file` task, dispatching on its `operation`.
fn build_file_op(fw: &FileWith, scope: &Scope<'_>) -> Result<FileOp, RunError> {
    let path = interp_to_string(&fw.path, scope)?;
    let contents = || -> Result<Vec<u8>, RunError> {
        Ok(fw
            .content
            .as_deref()
            .map(|c| interp_to_string(c, scope))
            .transpose()?
            .unwrap_or_default()
            .into_bytes())
    };
    match fw.operation.as_str() {
        "read" => Ok(FileOp::Read {
            path,
            encoding: fw.encoding.clone(),
        }),
        "write" => Ok(FileOp::Write {
            path,
            contents: contents()?,
        }),
        "append" => Ok(FileOp::Append {
            path,
            contents: contents()?,
        }),
        "delete" => Ok(FileOp::Delete { path }),
        "exists" => Ok(FileOp::Exists { path }),
        "copy" => Ok(FileOp::Copy {
            from: path,
            to: interp_destination(fw, scope)?,
        }),
        "move" => Ok(FileOp::Move {
            from: path,
            to: interp_destination(fw, scope)?,
        }),
        other => Err(RunError::validation(
            "unknown_file_operation",
            format!("unknown file operation {other:?}"),
        )),
    }
}

/// Interpolate the required `destination` of a `copy`/`move` file op.
fn interp_destination(fw: &FileWith, scope: &Scope<'_>) -> Result<String, RunError> {
    let destination = fw.destination.as_deref().ok_or_else(|| {
        RunError::validation(
            "file_missing_destination",
            "a copy/move file op needs a `destination`",
        )
    })?;
    interp_to_string(destination, scope)
}

/// Build a [`StoreOp`] for a `store` task, dispatching on its `operation`.
fn build_store_op(sw: &StoreWith, scope: &Scope<'_>) -> Result<StoreOp, RunError> {
    let key = || -> Result<String, RunError> {
        let key = sw.key.as_deref().ok_or_else(|| {
            RunError::validation("store_missing_key", "this store op needs a `key`")
        })?;
        interp_to_string(key, scope)
    };
    match sw.operation.as_str() {
        "get" => Ok(StoreOp::Get { key: key()? }),
        "put" => Ok(StoreOp::Put {
            key: key()?,
            body: sw
                .content
                .as_deref()
                .map(|c| interp_to_string(c, scope))
                .transpose()?
                .unwrap_or_default()
                .into_bytes(),
        }),
        "delete" => Ok(StoreOp::Delete { key: key()? }),
        "head" => Ok(StoreOp::Head { key: key()? }),
        "list" => Ok(StoreOp::List {
            prefix: sw
                .key
                .as_deref()
                .map(|p| interp_to_string(p, scope))
                .transpose()?
                .unwrap_or_default(),
        }),
        other => Err(RunError::validation(
            "unknown_store_operation",
            format!("unknown store operation {other:?}"),
        )),
    }
}

/// Build a [`ChatRequest`] for a `chat-completion` task.
fn build_chat_request(cw: &ChatCompletionWith, scope: &Scope<'_>) -> Result<ChatRequest, RunError> {
    let _ = scope;
    Ok(ChatRequest {
        model: cw.model.clone(),
        messages: cw.messages.clone(),
        temperature: cw.temperature,
        max_tokens: cw.max_tokens,
    })
}

// ---------------------------------------------------------------------------------------------
// Adapter-result → AdapterOutput normalisation helpers.
// ---------------------------------------------------------------------------------------------

/// Turn a process result into an [`AdapterOutput`], failing a non-zero / signalled exit.
///
/// Captured stdout that parses as JSON becomes a structured [`AdapterOutput::Json`]; otherwise the
/// raw bytes are handed on for text/blob normalisation.
fn process_output(
    out: crate::ports::driven::ProcessOutput,
    name: &str,
) -> Result<AdapterOutput, RunError> {
    if out.exit_code != Some(0) {
        let code = out
            .exit_code
            .map_or_else(|| "signal".to_string(), |c| c.to_string());
        return Err(RunError::run_failure(
            "exec_nonzero_exit",
            format!("task {name:?} exited with status {code}"),
        )
        .with_task(name));
    }
    Ok(bytes_output(out.stdout))
}

/// Normalise captured bytes: structured JSON stays JSON, otherwise text/blob wrapping applies.
fn bytes_output(bytes: Vec<u8>) -> AdapterOutput {
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) => AdapterOutput::Json(value),
        Err(_) => AdapterOutput::Bytes(bytes),
    }
}

/// Normalise a [`FileResult`] into an [`AdapterOutput`].
fn file_output(result: FileResult) -> AdapterOutput {
    match result {
        FileResult::Read { contents } => bytes_output(contents),
        FileResult::Exists { exists } => AdapterOutput::Json(Value::Bool(exists)),
        FileResult::Done => AdapterOutput::Json(serde_json::json!({ "ok": true })),
    }
}

/// Normalise a [`StoreResult`] into an [`AdapterOutput`].
fn store_output(result: StoreResult) -> AdapterOutput {
    match result {
        StoreResult::Get { body } => bytes_output(body),
        StoreResult::List { keys } => {
            AdapterOutput::Json(Value::Array(keys.into_iter().map(Value::String).collect()))
        }
        StoreResult::Head { exists, size_bytes } => AdapterOutput::Json(serde_json::json!({
            "exists": exists,
            "sizeBytes": size_bytes,
        })),
        StoreResult::Done => AdapterOutput::Json(serde_json::json!({ "ok": true })),
    }
}

/// Convert a schema [`Duration`] into [`Milliseconds`], parsing the `500ms`/`30s`/`5m`/`1h` string
/// form. An unparseable string yields `None` (no timeout) rather than an error.
fn duration_to_ms(duration: &Duration) -> Option<Milliseconds> {
    match duration {
        Duration::Seconds(seconds) => Some(Milliseconds(
            seconds.saturating_mul(MILLISECONDS_PER_SECOND),
        )),
        Duration::Spec(spec) => parse_duration_spec(spec).map(Milliseconds),
    }
}

/// Parse a `500ms`/`30s`/`5m`/`1h` duration string into milliseconds.
fn parse_duration_spec(spec: &str) -> Option<u64> {
    let spec = spec.trim();
    let (number, unit_ms) = if let Some(rest) = spec.strip_suffix("ms") {
        (rest, 1)
    } else if let Some(rest) = spec.strip_suffix('s') {
        (rest, MILLISECONDS_PER_SECOND)
    } else if let Some(rest) = spec.strip_suffix('m') {
        (rest, MILLISECONDS_PER_MINUTE)
    } else if let Some(rest) = spec.strip_suffix('h') {
        (rest, MILLISECONDS_PER_HOUR)
    } else {
        (spec, MILLISECONDS_PER_SECOND)
    };
    number
        .trim()
        .parse::<u64>()
        .ok()
        .map(|n| n.saturating_mul(unit_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_body_type_serialises_compact_json_with_the_json_content_type() {
        // An object body under the default `json` type is compact JSON and carries application/json.
        let (bytes, content_type) = serialize_fetch_body(&json!({"a": 1, "b": "x"}), "json");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"a":1,"b":"x"}"#,
            "a json body is compact JSON"
        );
        assert_eq!(
            content_type, "application/json",
            "the json content type is set"
        );
    }

    #[test]
    fn form_body_type_urlencodes_pairs_with_the_form_content_type() {
        // A form body becomes k=v&k2=v2 with the form content type; reserved chars are percent-encoded.
        let (bytes, content_type) =
            serialize_fetch_body(&json!({"name": "a b", "sym": "x&y"}), "form");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "name=a+b&sym=x%26y",
            "a form body is url-encoded, space→+ and & escaped"
        );
        assert_eq!(
            content_type, "application/x-www-form-urlencoded",
            "the form content type is set"
        );
    }

    #[test]
    fn text_and_binary_body_types_send_raw_bytes_with_their_content_types() {
        // Text keeps the raw string (no JSON quoting) and is text/plain; binary is octet-stream.
        let (text_bytes, text_ct) = serialize_fetch_body(&json!("hello world"), "text");
        assert_eq!(
            String::from_utf8(text_bytes).unwrap(),
            "hello world",
            "a text body is the raw string, not JSON-quoted"
        );
        assert_eq!(text_ct, "text/plain; charset=utf-8", "text content type");

        let (bin_bytes, bin_ct) = serialize_fetch_body(&json!("raw"), "binary");
        assert_eq!(bin_bytes, b"raw", "a binary body is the raw bytes");
        assert_eq!(bin_ct, "application/octet-stream", "binary content type");
    }

    #[test]
    fn an_unrecognised_body_type_falls_back_to_json_never_panics() {
        // Negative space: an out-of-enum bodyType (the schema constrains input, but the adapter is
        // defensive) falls back to JSON rather than panicking or dropping the body.
        let (bytes, content_type) = serialize_fetch_body(&json!({"k": "v"}), "totally-unknown");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"k":"v"}"#,
            "an unknown body type defaults to JSON"
        );
        assert_eq!(
            content_type, "application/json",
            "and the JSON content type"
        );
    }

    #[test]
    fn percent_encode_passes_unreserved_and_escapes_the_rest() {
        // Unreserved characters pass through; a space is '+'; other bytes are %XX (upper hex).
        assert_eq!(
            percent_encode("aZ0-_.~"),
            "aZ0-_.~",
            "unreserved characters are untouched"
        );
        assert_eq!(
            percent_encode("a b/c?"),
            "a+b%2Fc%3F",
            "space→+, and reserved bytes are percent-encoded uppercase"
        );
    }

    #[test]
    fn form_urlencode_of_a_non_object_encodes_a_single_field() {
        // Negative space: a mis-shaped (non-object) form body is still transmitted, percent-encoded,
        // not silently dropped.
        assert_eq!(
            form_urlencode(&json!("a=b c")),
            "a%3Db+c",
            "a non-object form body is a single encoded field"
        );
        assert_eq!(
            form_urlencode(&json!(42)),
            "42",
            "a number encodes verbatim"
        );
    }
}
