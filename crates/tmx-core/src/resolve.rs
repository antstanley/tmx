//! Engine-local resolution — turning a loaded [`Flow`] JSON value into the [`ResolvedFlow`] the
//! runner consumes.
//!
//! This is the *minimal* resolution the sequential runner needs: parse the source into the Task-03
//! [`Flow`] model, desugar its task collection into an ordered `Vec<`[`Task`]`>` (map-form keys
//! become task names, string shorthands become `exec` tasks), inline the environment/context, and
//! fold declared-input defaults into the supplied inputs. Full loader/preflight resolution —
//! reference chasing for `context`/`environment`, cycle detection, capability checks — is a separate
//! loading unit (03/14); a *reference* here is therefore an explicit `unsupported_reference` error
//! rather than a silent drop, so nothing is quietly skipped.

use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::flow::{ContextRef, EnvironmentRef, TaskEntry, Tasks};
use tmx_schema::task::{ExecWith, Task, TaskWith};
use tmx_schema::{Flow, InputSpec};

use crate::error::RunError;
use crate::model::ResolvedFlow;

/// Parse a loaded source [`Value`] into a [`Flow`] and resolve it into a [`ResolvedFlow`].
///
/// A malformed document is a typed `flow_parse_error`; an unsupported *reference* form
/// (`context`/`environment` given as a path rather than inline) is `unsupported_reference`.
pub fn resolve_flow(source: Value) -> Result<ResolvedFlow, RunError> {
    let flow: Flow = serde_json::from_value(source).map_err(|e| {
        RunError::validation("flow_parse_error", format!("could not parse the flow: {e}"))
    })?;
    resolve(flow)
}

/// Resolve an already-parsed [`Flow`].
pub fn resolve(flow: Flow) -> Result<ResolvedFlow, RunError> {
    let environment = match flow.environment {
        None => None,
        Some(EnvironmentRef::Inline(env)) => Some(*env),
        Some(EnvironmentRef::Reference(_)) => {
            return Err(RunError::resolution(
                "unsupported_reference",
                "a referenced environment is resolved by the loading unit, not the runner",
            ));
        }
    };
    let context = match flow.context {
        None => None,
        Some(ContextRef::Inline(ctx)) => Some(*ctx),
        Some(ContextRef::Reference(_)) => {
            return Err(RunError::resolution(
                "unsupported_reference",
                "a referenced context is resolved by the loading unit, not the runner",
            ));
        }
    };
    let inputs = flow.inputs.unwrap_or_default();
    let tasks = desugar_tasks(flow.tasks)?;
    Ok(ResolvedFlow {
        name: flow.name,
        description: flow.description,
        version: flow.version,
        environment,
        context,
        inputs,
        tasks,
    })
}

/// Desugar a [`Tasks`] collection into an ordered `Vec<`[`Task`]`>`: the array form is taken as-is,
/// the map form fills each task's `name` from its key (a string entry desugars to an `exec` task).
pub(crate) fn desugar_tasks(tasks: Tasks) -> Result<Vec<Task>, RunError> {
    match tasks {
        Tasks::List(list) => Ok(list),
        Tasks::Map(map) => Ok(map
            .into_iter()
            .map(|(name, entry)| match entry {
                TaskEntry::Task(mut task) => {
                    task.name = Some(name);
                    *task
                }
                TaskEntry::Shorthand(command) => exec_task(name, command),
            })
            .collect()),
    }
}

/// Build a full `exec` [`Task`] named `name` running `command` — the desugaring of a map-form string
/// shorthand.
fn exec_task(name: String, command: String) -> Task {
    Task {
        kind: None,
        name: Some(name),
        description: None,
        if_condition: None,
        secrets: None,
        context: None,
        context_strategy: None,
        context_precedence: None,
        output: None,
        produces: None,
        continue_on_error: None,
        with: TaskWith::Exec(ExecWith {
            command,
            args: None,
            shell: None,
            cwd: None,
            env: None,
            timeout: None,
        }),
    }
}

/// Fold declared-input defaults into the supplied `inputs`, producing the `inputs.*` scope object.
///
/// The supplied object wins; any declared input it omits contributes its `default` when it has one.
/// A non-object `supplied` is treated as "no inputs supplied".
#[must_use]
pub fn merged_inputs(supplied: &Value, specs: &IndexMap<String, InputSpec>) -> Value {
    let mut merged = supplied
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    for (name, spec) in specs {
        if !merged.contains_key(name)
            && let Some(default) = &spec.default
        {
            merged.insert(name.clone(), default.clone());
        }
    }
    Value::Object(merged)
}
