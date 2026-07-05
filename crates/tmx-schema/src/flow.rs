//! `Flow` and the top-level input entities. Deserialise-only mirror of the `flow`, `inputSpec`,
//! `tasks`, and `taskList` `$def`s in [`docs/tmx.schema.json`](../../../docs/tmx.schema.json).
//!
//! A [`Flow`] is the static definition a runtime turns into a Pipeline: an optional environment and
//! context (each inline or referenced), declared inputs, and the required task collection. The task
//! collection is either an ordered array or a name-keyed map ([`Tasks`]); both run in source order,
//! which the map form preserves by deserialising into an [`indexmap::IndexMap`] rather than an
//! unordered map — key order is a type property here, not a parser accident.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::context::Context;
use crate::environment::Environment;
use crate::task::Task;

/// The `flow` `$def`: a static definition of tasks plus optional context and environment.
///
/// `additionalProperties: false` and only `tasks` is required. `kind` is the optional artifact
/// discriminator (the constant `"flow"`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Flow {
    /// The tasks the Flow runs, as an ordered array or a name-keyed map.
    pub tasks: Tasks,
    /// Optional artifact discriminator; the constant `"flow"` for a top-level document.
    #[serde(default)]
    pub kind: Option<String>,
    /// Human-readable name of the Flow.
    #[serde(default)]
    pub name: Option<String>,
    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional version/identifier for this Flow definition.
    #[serde(default)]
    pub version: Option<String>,
    /// Runtime environment — inline object or a path/name reference.
    #[serde(default)]
    pub environment: Option<EnvironmentRef>,
    /// Context (env, secrets, hooks) — inline object or a path/name reference.
    #[serde(default)]
    pub context: Option<ContextRef>,
    /// Declared input variables, keyed by name, in source order.
    #[serde(default)]
    pub inputs: Option<IndexMap<String, InputSpec>>,
}

/// An inline [`Environment`] or a string reference to a standalone one (`flow.environment`
/// `oneOf [environment, reference]`). The inline object is boxed to keep [`Flow`] compact.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum EnvironmentRef {
    /// An inline environment definition.
    Inline(Box<Environment>),
    /// A path/name reference to a standalone environment.
    Reference(String),
}

/// An inline [`Context`] or a string reference to a standalone one (`flow.context` /`task.context`
/// `oneOf [context, reference]`). The inline object is boxed to keep the parent compact.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ContextRef {
    /// An inline context definition.
    Inline(Box<Context>),
    /// A path/name reference to a standalone context.
    Reference(String),
}

/// The `inputSpec` `$def`: the declaration of one Flow input variable.
/// `additionalProperties: false`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputSpec {
    /// Expected JSON type: `string`, `number`, `boolean`, `object`, or `array`.
    #[serde(default, rename = "type")]
    pub input_type: Option<String>,
    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,
    /// When true, the input must be supplied at invocation; defaults to false.
    #[serde(default)]
    pub required: Option<bool>,
    /// Value used when the input is not supplied; any JSON type.
    #[serde(default)]
    pub default: Option<Value>,
}

/// The `tasks` `$def`: the task collection, as EITHER an ordered array (`taskList`) OR a name-keyed
/// map.
///
/// Untagged: a JSON array deserialises to [`Tasks::List`] and a JSON object to [`Tasks::Map`]. The
/// map form is an [`indexmap::IndexMap`], so its keys keep the source document's order — the whole
/// point of preserving "map form runs in key order" as a type guarantee. Each map value is a
/// [`TaskEntry`] — a full task object or an `exec` string shorthand.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Tasks {
    /// The ordered array form (`taskList`): each item is a full task object.
    List(Vec<Task>),
    /// The name-keyed map form: keys are task names, values are tasks or `exec` shorthands.
    Map(IndexMap<String, TaskEntry>),
}

/// A value in the map form of [`Tasks`]: a full task object, or a string shorthand for an `exec`
/// task whose command is that string (`tasks` map `additionalProperties oneOf [task, string]`).
///
/// Untagged with [`TaskEntry::Task`] first: an object deserialises to a boxed [`Task`]; a string
/// falls through to [`TaskEntry::Shorthand`]. The shapes are disjoint, so the order only reflects
/// intent. The task is boxed because [`Task`] is comparatively large.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum TaskEntry {
    /// A full task object (its `name` may be omitted — the map key is the name).
    Task(Box<Task>),
    /// A string shorthand for an `exec` task running that shell command.
    Shorthand(String),
}

impl Tasks {
    /// The task names in source order: the array form yields each task's own `name` (or `None`
    /// when omitted), the map form yields its keys in insertion (document) order.
    ///
    /// This is the order-preservation observation the corpus test asserts against — both forms are
    /// projected to the same ordered name sequence so array-form and map-form order can be checked
    /// uniformly.
    #[must_use]
    pub fn names_in_order(&self) -> Vec<Option<&str>> {
        match self {
            Tasks::List(tasks) => tasks.iter().map(|t| t.name.as_deref()).collect(),
            Tasks::Map(map) => map.keys().map(|k| Some(k.as_str())).collect(),
        }
    }

    /// The number of tasks in the collection, in either form.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Tasks::List(tasks) => tasks.len(),
            Tasks::Map(map) => map.len(),
        }
    }

    /// Whether the collection has no tasks. The schema forbids this (`minItems`/`minProperties` of
    /// 1), but the accessor is total so callers need not special-case the form.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_form_accessors_report_order_and_size() {
        let flow: Flow = serde_json::from_str(
            r#"{ "tasks": [
                { "name": "first", "type": "exec", "with": { "command": "a" } },
                { "type": "exec", "with": { "command": "b" } }
            ] }"#,
        )
        .expect("array-form flow deserialises");

        assert!(matches!(flow.tasks, Tasks::List(_)), "array form is List");
        assert_eq!(flow.tasks.len(), 2, "two array-form tasks");
        assert!(!flow.tasks.is_empty(), "a two-task list is not empty");
        // The first task names itself; the second omits `name` (still legal in the array form).
        assert_eq!(
            flow.tasks.names_in_order(),
            vec![Some("first"), None],
            "array-form names follow item order, with an unnamed task as None"
        );
    }

    #[test]
    fn map_form_accessors_report_key_order_and_size() {
        // Keys deliberately not in sorted order, so the accessor's order can only match under an
        // insertion-ordered map.
        let flow: Flow =
            serde_json::from_str(r#"{ "tasks": { "gamma": "g", "beta": "b", "alpha": "a" } }"#)
                .expect("map-form flow deserialises");

        assert!(matches!(flow.tasks, Tasks::Map(_)), "map form is Map");
        assert_eq!(flow.tasks.len(), 3, "three map-form tasks");
        assert!(!flow.tasks.is_empty(), "a three-task map is not empty");
        assert_eq!(
            flow.tasks.names_in_order(),
            vec![Some("gamma"), Some("beta"), Some("alpha")],
            "map-form names follow source key order, not sorted order"
        );
    }
}
