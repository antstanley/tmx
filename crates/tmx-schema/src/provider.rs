//! Provider manifest types — a deserialise-only mirror of the `provider` `$def`s in
//! [`docs/tmx-provider.schema.json`](../../../docs/tmx-provider.schema.json).
//!
//! An Environment Provider materialises a Flow's `environment` block into real resources. It is
//! either a standalone **binary** the core CLI invokes with a per-method subcommand, or a **flow**
//! whose method bodies are ordinary TMX task collections run through the same `PipelineRunner`
//! ([`.specs/06-ports-and-adapters.md` §Environment and provider execution](../../../.specs/06-ports-and-adapters.md)).
//! Every provider must implement all four lifecycle methods (`bootstrap`/`deploy`/`clean`/`destroy`).
//!
//! This crate models only the *shape* of a manifest; interpreting a method body (invoking a binary,
//! or running a task collection as a Flow) is the `EnvironmentProvider` adapter's concern in
//! `tmx-adapters`. A method body carrying inline tasks reuses the same [`Tasks`] type a Flow's task
//! list does, so a provider method inherits the entire task model automatically.

use serde::Deserialize;
use serde_json::Value;

use crate::context::HookUse;
use crate::flow::Tasks;

/// How a provider is implemented — the manifest's `type` field.
///
/// `binary` — a standalone executable the core CLI invokes with each method's subcommand string;
/// `flow` — a Flow whose task collections implement each method. Serialises `lowercase`, matching
/// the schema enum exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    /// A standalone executable invoked by the core CLI (`type: binary`, requires `binary`).
    Binary,
    /// A Flow whose task collections implement each lifecycle method (`type: flow`).
    Flow,
}

/// One provider method's implementation body — the `method` `$def`'s `oneOf`.
///
/// Untagged, tried in an order that resolves the object/object ambiguity, exactly as
/// [`Hook`](crate::context::Hook) does: a string is a [`Ref`](ProviderMethodBody::Ref); an object
/// bearing the required `use` key is a [`Use`](ProviderMethodBody::Use) import; any other object or
/// array is an inline [`Tasks`](ProviderMethodBody::Tasks) collection. [`Use`](ProviderMethodBody::Use)
/// must precede [`Tasks`](ProviderMethodBody::Tasks) so a `{ use, inputs }` object is not misread as
/// a one-key task map named `use`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ProviderMethodBody {
    /// A subcommand string (for a `binary` provider) or a Flow reference (for a `flow` provider).
    Ref(String),
    /// A `{ use, inputs }` import of a Flow that implements this method.
    Use(HookUse),
    /// An inline task collection (an ordered array, or a name-keyed map), run as a Flow.
    Tasks(Tasks),
}

/// The four lifecycle methods every provider must implement — the manifest's `methods` object.
///
/// `additionalProperties: false` and all four required, so a manifest missing one, or carrying a
/// stray key, fails to deserialise (the mirror's negative space).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMethods {
    /// Bootstrap the environment substrate (provision networks, create clusters).
    pub bootstrap: ProviderMethodBody,
    /// Bring an ephemeral environment up for a Flow run.
    pub deploy: ProviderMethodBody,
    /// Tear a deployed ephemeral environment down (best-effort teardown).
    pub clean: ProviderMethodBody,
    /// Destroy the entire substrate, including everything `bootstrap` created.
    pub destroy: ProviderMethodBody,
}

/// A provider manifest — the top-level `tmx-provider.schema.json` document.
///
/// `additionalProperties: false`; `name`, `type`, and `methods` are required. `binary` is required
/// (by the schema's conditional) when `type` is `binary`; this deserialise-only mirror keeps it
/// optional and the adapter enforces its presence at invocation, so a malformed manifest is a typed
/// runtime error naming the provider rather than a deserialisation failure with no context.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderManifest {
    /// Optional artifact discriminator; the constant `"provider"` for a manifest file.
    #[serde(default)]
    pub kind: Option<String>,
    /// Provider name, as referenced by an environment's `provider` field.
    pub name: String,
    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional version/identifier for this manifest.
    #[serde(default)]
    pub version: Option<String>,
    /// Platform this provider targets (`local`, `aws`, `gcp`, …).
    #[serde(default)]
    pub platform: Option<String>,
    /// How the provider is implemented (`binary` | `flow`).
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    /// Path or name of the executable (used when `type` is `binary`).
    #[serde(default)]
    pub binary: Option<String>,
    /// Optional JSON Schema (inline object or a `$ref`/path string) describing the provider-specific
    /// `environment.options` this provider accepts.
    #[serde(default)]
    pub options_schema: Option<Value>,
    /// The four lifecycle methods.
    pub methods: ProviderMethods,
}

impl ProviderMethods {
    /// The body of the method named `name` (`bootstrap`/`deploy`/`clean`/`destroy`), or `None` for
    /// any other name — total over the closed method vocabulary.
    #[must_use]
    pub fn body(&self, name: &str) -> Option<&ProviderMethodBody> {
        match name {
            "bootstrap" => Some(&self.bootstrap),
            "deploy" => Some(&self.deploy),
            "clean" => Some(&self.clean),
            "destroy" => Some(&self.destroy),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_manifest_with_inline_task_bodies_deserialises() {
        // A minimal `flow` provider: each method is an inline ordered task array. The bodies land in
        // the shared `Tasks` type, so a provider method inherits the whole task model.
        let json = serde_json::json!({
            "kind": "provider",
            "name": "local",
            "type": "flow",
            "methods": {
                "bootstrap": [ { "name": "b", "type": "exec", "with": { "command": "echo boot" } } ],
                "deploy": [ { "name": "d", "type": "exec", "with": { "command": "echo deploy" } } ],
                "clean": [ { "name": "c", "type": "exec", "with": { "command": "echo clean" } } ],
                "destroy": [ { "name": "x", "type": "exec", "with": { "command": "echo destroy" } } ]
            }
        });
        let manifest: ProviderManifest =
            serde_json::from_value(json).expect("a flow manifest deserialises");
        assert_eq!(manifest.provider_type, ProviderType::Flow, "type is flow");
        assert_eq!(manifest.name, "local", "the name is captured");
        assert!(
            matches!(manifest.methods.deploy, ProviderMethodBody::Tasks(_)),
            "an inline task-array body is the Tasks variant"
        );
        // `body` is total over the closed method vocabulary and rejects anything else.
        assert!(manifest.methods.body("deploy").is_some(), "deploy resolves");
        assert!(
            manifest.methods.body("nonsense").is_none(),
            "an out-of-vocabulary method name resolves to None, not a panic"
        );
    }

    #[test]
    fn a_binary_manifest_with_subcommand_bodies_deserialises() {
        // A `binary` provider: each method is a subcommand string, and `binary` names the executable.
        let json = serde_json::json!({
            "name": "ecs",
            "type": "binary",
            "binary": "tmx-ecs",
            "optionsSchema": { "type": "object", "required": ["cluster"] },
            "methods": {
                "bootstrap": "bootstrap",
                "deploy": "deploy",
                "clean": "clean",
                "destroy": "destroy"
            }
        });
        let manifest: ProviderManifest =
            serde_json::from_value(json).expect("a binary manifest deserialises");
        assert_eq!(
            manifest.provider_type,
            ProviderType::Binary,
            "type is binary"
        );
        assert_eq!(
            manifest.binary.as_deref(),
            Some("tmx-ecs"),
            "binary captured"
        );
        assert!(
            matches!(manifest.methods.deploy, ProviderMethodBody::Ref(ref s) if s == "deploy"),
            "a string body is the Ref (subcommand) variant"
        );
        assert!(
            manifest.options_schema.is_some(),
            "the optionsSchema is retained for preflight validation"
        );
    }

    #[test]
    fn a_use_import_body_is_not_misread_as_a_task_map() {
        // The `{ use, inputs }` import form must win over the name-keyed task-map form: `Use` precedes
        // `Tasks` in the untagged enum, exactly as `Hook` orders them.
        let body: ProviderMethodBody = serde_json::from_value(
            serde_json::json!({ "use": "./deploy.yaml", "inputs": { "n": 1 } }),
        )
        .expect("a use-import body deserialises");
        match body {
            ProviderMethodBody::Use(import) => {
                assert_eq!(
                    import.use_ref, "./deploy.yaml",
                    "the use reference is captured"
                );
                assert!(import.inputs.is_some(), "the import inputs are captured");
            }
            other => panic!("a {{ use, inputs }} body must be the Use variant, got {other:?}"),
        }
    }

    #[test]
    fn a_manifest_missing_a_required_method_is_rejected() {
        // Negative space: `methods` requires all four; a manifest missing `destroy` fails to
        // deserialise rather than silently modelling three methods.
        let json = serde_json::json!({
            "name": "broken",
            "type": "flow",
            "methods": {
                "bootstrap": "b",
                "deploy": "d",
                "clean": "c"
            }
        });
        assert!(
            serde_json::from_value::<ProviderManifest>(json).is_err(),
            "a manifest missing a required lifecycle method must not deserialise"
        );
    }
}
