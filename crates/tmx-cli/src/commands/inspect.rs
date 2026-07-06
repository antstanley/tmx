//! `tmx inspect` — the resolved-plan projection behind [`InspectFlow`] (07 §`tmx inspect`).
//!
//! Resolves the Flow by the same search order as `tmx run`, preflights it (fail-fast validation +
//! the capability check), then projects the runner-ready [`Preflighted`] flow into one JSON view:
//! the resolved environment and context, the ordered task plan, the declared inputs, the required
//! capabilities, and the secrets the Flow needs — **every secret masked**, never a raw value. The
//! same projection backs `tmx context show` (the env + masked secrets slice) and `tmx secrets list`
//! (the secrets-needed slice), so all three mask on one path.
//!
//! A malformed artifact aborts here (preflight's typed `validation` error → exit 3) before any
//! projection is printed — the fail-fast the certificate's O2 negative space names.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use tmx_adapters::sink::Format;

use tmx_core::ports::driving::{InspectFlow, Inspection};
use tmx_core::{Masker, Preflighted, RunError, preflight};
use tmx_schema::task::TaskWith;
use tmx_schema::{Environment, SecretSource, SecretValue};

use crate::args::RunArgs;
use crate::commands::run::resolve_target;
use crate::compose::Composed;
use crate::config;

/// The placeholder a literal secret is projected as — a fixed marker so nothing about the value
/// leaks. The whole projection is additionally routed through the [`Masker`] before it is returned,
/// so an accidental secret substring anywhere is scrubbed too.
const MASKED_SECRET_PLACEHOLDER: &str = "[REDACTED]";

/// The `InspectFlow` use case over the built-in composition: resolve → preflight → project.
pub struct EngineInspectFlow;

#[async_trait]
impl InspectFlow for EngineInspectFlow {
    async fn inspect(&self, reference: &str) -> Result<Inspection, RunError> {
        let preflighted = resolve_and_preflight(Some(reference), None).await?;
        Ok(Inspection {
            view: project(&preflighted),
        })
    }
}

/// Resolve a Flow (positional `flow` / `--file`, else the `tmx run` search order) and preflight it,
/// returning the runner-ready [`Preflighted`]. A malformed artifact is a fail-fast typed error.
///
/// # Errors
///
/// Returns `resolution` for an unresolved Flow, or `validation` (exit 3) for a malformed artifact or
/// a breached limit — surfaced before any projection is built.
pub async fn resolve_and_preflight(
    flow: Option<&str>,
    file: Option<&str>,
) -> Result<Preflighted, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let run_args = RunArgs {
        flow: flow.map(str::to_string),
        file: file.map(str::to_string),
        ..RunArgs::default()
    };
    let resolved = resolve_target(&run_args, &cwd, config::env_flow())?;
    // Inspect reaches no effecting port; only the resolve → load → validate ports matter, so the
    // format/colour/store surface is irrelevant.
    let composed = Composed::new(resolved.base_dir.clone(), Format::Json, false, None)?;
    let preflighted = preflight(
        &resolved.target,
        composed.preflight_ports(),
        &composed.available_capabilities(),
    )
    .await?;
    Ok(preflighted)
}

/// Project a preflighted Flow into the full inspection view, with every secret masked. The result is
/// routed through a [`Masker`] registered with the Flow's literal secrets, so no raw secret survives.
#[must_use]
pub fn project(preflighted: &Preflighted) -> Value {
    let flow = &preflighted.flow;
    let mut masker = Masker::new();
    register_literal_secrets(flow.context.as_ref(), &mut masker);

    let view = json!({
        "flow": flow.name.clone(),
        "environment": environment_projection(flow.environment.as_ref()),
        "context": context_projection(flow.context.as_ref()),
        "tasks": task_plan(&flow.tasks),
        "inputs": input_projection(flow),
        "capabilities": capability_list(preflighted),
        "secretsNeeded": secrets_needed(flow.context.as_ref()),
    });
    // Defence in depth: scrub the whole projection through the Masker so any registered secret that
    // slipped into a field (an env value, a description) is redacted before it reaches stdout.
    masker.redact_value(&view).into_inner().into_owned()
}

/// The context slice of the projection: env vars and masked secrets (the `tmx context show` view).
#[must_use]
pub fn context_projection(context: Option<&tmx_schema::Context>) -> Value {
    let Some(context) = context else {
        return Value::Null;
    };
    let mut env = Map::new();
    if let Some(map) = &context.env {
        for (key, value) in map {
            env.insert(key.clone(), Value::String(value.clone()));
        }
    }
    let mut secrets = Map::new();
    if let Some(map) = &context.secrets {
        for (name, value) in map {
            secrets.insert(name.clone(), masked_secret(value));
        }
    }
    json!({ "env": Value::Object(env), "secrets": Value::Object(secrets) })
}

/// The masked projection of one secret value — never the raw value. A literal secret becomes the
/// placeholder; a structured source shows only its non-secret descriptor (the env-var name, file
/// path, provider, and key), which name *where* the value comes from, not the value itself.
fn masked_secret(value: &SecretValue) -> Value {
    match value {
        SecretValue::Literal(_) => json!({
            "kind": "literal",
            "value": MASKED_SECRET_PLACEHOLDER,
        }),
        SecretValue::Source(source) => {
            let mut descriptor = Map::new();
            descriptor.insert(
                "kind".to_string(),
                Value::String(source_kind(source).to_string()),
            );
            if let Some(env) = &source.env {
                descriptor.insert("env".to_string(), Value::String(env.clone()));
            }
            if let Some(file) = &source.file {
                descriptor.insert("file".to_string(), Value::String(file.clone()));
            }
            if let Some(provider) = &source.provider {
                descriptor.insert("provider".to_string(), Value::String(provider.clone()));
            }
            if let Some(key) = &source.key {
                descriptor.insert("key".to_string(), Value::String(key.clone()));
            }
            // The value is deliberately absent — a source secret is resolved at run time, never here.
            Value::Object(descriptor)
        }
    }
}

/// The secrets-needed list (the `tmx secrets list` view): one masked entry per declared secret,
/// naming the secret and its source kind — never a value.
#[must_use]
pub fn secrets_needed(context: Option<&tmx_schema::Context>) -> Value {
    let Some(context) = context else {
        return Value::Array(Vec::new());
    };
    let Some(map) = &context.secrets else {
        return Value::Array(Vec::new());
    };
    let entries: Vec<Value> = map
        .iter()
        .map(|(name, value)| {
            let kind = match value {
                SecretValue::Literal(_) => "literal",
                SecretValue::Source(source) => source_kind(source),
            };
            json!({ "name": name, "source": kind, "value": MASKED_SECRET_PLACEHOLDER })
        })
        .collect();
    Value::Array(entries)
}

/// The source kind of a structured secret: `env`, `file`, or `provider` (the first that is set),
/// else `unknown` when the descriptor names none.
fn source_kind(source: &SecretSource) -> &'static str {
    if source.env.is_some() {
        "env"
    } else if source.file.is_some() {
        "file"
    } else if source.provider.is_some() {
        "provider"
    } else {
        "unknown"
    }
}

/// Register every literal secret value with the Masker, so the whole-projection scrub scrubs any
/// echo of it. A structured source carries no literal value to register.
fn register_literal_secrets(context: Option<&tmx_schema::Context>, masker: &mut Masker) {
    if let Some(context) = context
        && let Some(secrets) = &context.secrets
    {
        for value in secrets.values() {
            if let SecretValue::Literal(literal) = value {
                masker.register(literal.clone());
            }
        }
    }
}

/// The environment slice: the resolved [`Environment`] as JSON (its `bootstrap` tasks are omitted by
/// the schema mirror's `Serialize`), or null when the Flow declares no environment.
fn environment_projection(environment: Option<&Environment>) -> Value {
    environment.map_or(Value::Null, |env| {
        serde_json::to_value(env).unwrap_or(Value::Null)
    })
}

/// The ordered task plan: one `{ name, type }` per task, in execution order.
fn task_plan(tasks: &[tmx_schema::task::Task]) -> Value {
    let entries: Vec<Value> = tasks
        .iter()
        .map(|task| {
            json!({
                "name": task.name.clone().unwrap_or_default(),
                "type": task_type_name(&task.with),
            })
        })
        .collect();
    Value::Array(entries)
}

/// The declared-inputs slice: name → `{ type, required, default, description }`. Built by hand
/// because [`InputSpec`](tmx_schema::InputSpec) is deserialise-only.
fn input_projection(flow: &tmx_core::ResolvedFlow) -> Value {
    let mut inputs = Map::new();
    for (name, spec) in &flow.inputs {
        inputs.insert(
            name.clone(),
            json!({
                "type": spec.input_type.clone(),
                "required": spec.required.unwrap_or(false),
                "default": spec.default.clone(),
                "description": spec.description.clone(),
            }),
        );
    }
    Value::Object(inputs)
}

/// The required capabilities, as their stable machine tokens, in requirement order.
fn capability_list(preflighted: &Preflighted) -> Value {
    let tokens: Vec<Value> = preflighted
        .capabilities
        .requirements()
        .map(|(capability, _)| Value::String(capability.as_str().to_string()))
        .collect();
    Value::Array(tokens)
}

/// The stable `type` token for a task's `with` payload — exhaustive over the closed [`TaskWith`]
/// vocabulary, so a new variant forces an update here.
#[must_use]
pub fn task_type_name(with: &TaskWith) -> &'static str {
    match with {
        TaskWith::Exec(_) => "exec",
        TaskWith::Run(_) => "run",
        TaskWith::Fetch(_) => "fetch",
        TaskWith::File(_) => "file",
        TaskWith::Store(_) => "store",
        TaskWith::ChatCompletion(_) => "chat-completion",
        TaskWith::Assert(_) => "assert",
        TaskWith::Map(_) => "map",
        TaskWith::Eval(_) => "eval",
        TaskWith::Flow(_) => "flow",
    }
}

/// Run `tmx inspect`, returning the full resolved-plan projection as one JSON object.
///
/// Resolves the Flow reference (positional / `--file` / search order) to a concrete path, then drives
/// the [`EngineInspectFlow`] use case — the same `InspectFlow` port a library/HTTP host would call.
///
/// # Errors
///
/// Returns `resolution` for an unresolved Flow or `validation` (exit 3) for a malformed artifact,
/// fail-fast before any projection is printed.
pub async fn execute(args: crate::args::InspectArgs) -> Result<Value, RunError> {
    let reference = resolve_reference(args.flow.as_deref(), args.file.as_deref())?;
    let inspection = EngineInspectFlow.inspect(&reference).await?;
    Ok(inspection.view)
}

/// Resolve a Flow reference (positional / `--file` / the `tmx run` search order) to a concrete path
/// — a single file, or a directory layout's directory — for the `InspectFlow` port to consume.
fn resolve_reference(flow: Option<&str>, file: Option<&str>) -> Result<String, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    // A positional Flow that names a registered-name → path mapping resolves to its mapped path.
    let flow = flow.map(|name| config::resolve_registered(config::ConfigLayer::new(), &cwd, name));
    let run_args = RunArgs {
        flow,
        file: file.map(str::to_string),
        ..RunArgs::default()
    };
    let resolved = resolve_target(&run_args, &cwd, config::env_flow())?;
    Ok(resolved
        .file_reference
        .unwrap_or_else(|| resolved.base_dir.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmx_core::resolve_flow;

    /// Preflight a JSON flow value into a `Preflighted` for the projection tests (no effecting port
    /// is needed, so the resolved flow is wrapped directly with an empty capability set).
    fn preflighted(flow: Value) -> Preflighted {
        Preflighted {
            flow: resolve_flow(flow).expect("the fixture flow resolves"),
            capabilities: tmx_core::CapabilitySet::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn projection_masks_a_literal_secret_and_never_emits_the_raw_value() {
        // A literal secret is projected as the placeholder; the raw value appears nowhere in the view.
        let raw = "super-secret-token-value";
        let view = project(&preflighted(json!({
            "context": { "secrets": { "TOKEN": raw } },
            "tasks": [ { "name": "a", "type": "exec", "with": { "command": "noop" } } ]
        })));
        let rendered = serde_json::to_string(&view).expect("the view renders");
        assert!(
            !rendered.contains(raw),
            "the raw secret value must never reach the projection, got {rendered}"
        );
        assert!(
            rendered.contains(MASKED_SECRET_PLACEHOLDER),
            "the secret is projected as the masked placeholder"
        );
        // The secrets-needed slice names the secret and its source, still masked.
        let needed = &view["secretsNeeded"];
        assert_eq!(needed[0]["name"], json!("TOKEN"), "the secret is named");
        assert_eq!(needed[0]["source"], json!("literal"), "its source kind");
        assert_eq!(
            needed[0]["value"],
            json!(MASKED_SECRET_PLACEHOLDER),
            "the needed entry carries only the masked value"
        );
    }

    #[test]
    fn projection_lists_the_ordered_plan_and_declared_inputs() {
        // The plan preserves task order with types, and inputs carry their declared shape.
        let view = project(&preflighted(json!({
            "name": "demo",
            "inputs": { "count": { "type": "number", "required": true, "default": 1 } },
            "tasks": [
                { "name": "first", "type": "exec", "with": { "command": "one" } },
                { "name": "second", "type": "exec", "with": { "command": "two" } }
            ]
        })));
        assert_eq!(view["flow"], json!("demo"), "the flow name projects");
        assert_eq!(view["tasks"][0]["name"], json!("first"), "order preserved");
        assert_eq!(
            view["tasks"][1]["type"],
            json!("exec"),
            "task type projected"
        );
        assert_eq!(
            view["inputs"]["count"]["type"],
            json!("number"),
            "the input's declared type projects"
        );
        assert_eq!(
            view["inputs"]["count"]["required"],
            json!(true),
            "a required input is marked required"
        );
    }

    #[test]
    fn source_secret_projects_its_descriptor_without_a_value() {
        // A structured source secret shows where it comes from, never a value field.
        let view = project(&preflighted(json!({
            "context": { "secrets": { "DB": { "env": "DATABASE_URL" } } },
            "tasks": [ { "name": "a", "type": "exec", "with": { "command": "noop" } } ]
        })));
        let projected = &view["context"]["secrets"]["DB"];
        assert_eq!(projected["kind"], json!("env"), "the source kind is env");
        assert_eq!(
            projected["env"],
            json!("DATABASE_URL"),
            "the env-var name (not a secret) is shown"
        );
        assert!(
            projected.get("value").is_none(),
            "a source secret carries no value field"
        );
    }
}
