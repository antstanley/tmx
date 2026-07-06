//! `Context` and its parts — the environment variables, secrets, and lifecycle hooks a Flow's
//! tasks run with. Deserialise-only mirror of the `context`, `hook`, `secretSource`, and `envMap`
//! `$def`s in [`docs/tmx.schema.json`](../../../docs/tmx.schema.json).
//!
//! A Context is reusable: it may be inlined in a Flow/task or defined standalone (`context.*`) and
//! referenced by path. Every object that carries user-authored keys uses [`indexmap::IndexMap`] so
//! source order survives deserialisation as a type property, not an incidental parser detail.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

/// A map of environment-variable names to string values (`envMap` `$def`). Ordered so the source
/// order of the keys is preserved. Values may embed `${{ … }}` interpolation; that is the
/// interpolator's concern, not this model's — here they are opaque strings.
pub type EnvMap = IndexMap<String, String>;

/// The `context` `$def`: the env vars, secrets, and lifecycle hooks available to a Flow's tasks.
///
/// `additionalProperties: false` in the schema — every field is named, so the mirror rejects an
/// unknown key rather than silently dropping it.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Context {
    /// Optional artifact discriminator; the constant `"context"` for a standalone `context.*` file.
    #[serde(default)]
    pub kind: Option<String>,
    /// Human-readable name of the Context.
    #[serde(default)]
    pub name: Option<String>,
    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,
    /// Environment variables exposed to tasks, in source order.
    #[serde(default)]
    pub env: Option<EnvMap>,
    /// Named secrets, each a literal/interpolated string or a structured [`SecretSource`].
    #[serde(default)]
    pub secrets: Option<IndexMap<String, SecretValue>>,
    /// Lifecycle hooks that fire over the course of a Pipeline.
    #[serde(default)]
    pub hooks: Option<Hooks>,
}

/// A secret value: either a literal/interpolated string, or a structured [`SecretSource`]
/// descriptor (`context.secrets.*` `oneOf [string, secretSource]`).
///
/// Untagged: a JSON string deserialises to [`SecretValue::Literal`]; an object to
/// [`SecretValue::Source`]. The two shapes are disjoint, so the match is unambiguous.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum SecretValue {
    /// A literal or `${{ … }}`-interpolated string.
    Literal(String),
    /// A structured descriptor naming where the value is read from.
    Source(SecretSource),
}

/// The `secretSource` `$def`: a structured descriptor for where a secret is read from.
///
/// `additionalProperties: true` — provider-specific keys beyond the named ones are captured in
/// [`SecretSource::extra`] so nothing is lost.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretSource {
    /// Read the value from a host environment variable of this name.
    #[serde(default)]
    pub env: Option<String>,
    /// Read the value from this file path.
    #[serde(default)]
    pub file: Option<String>,
    /// Named secret provider, e.g. `aws-sm`, `gcp-sm`, `vault`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Key/path within the secret provider.
    #[serde(default)]
    pub key: Option<String>,
    /// Any additional provider-specific keys (`additionalProperties: true`), in source order.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// The `context.hooks` object: a hook body per Pipeline-lifecycle transition
/// (`create`/`change`/`destroy`/`error`). `additionalProperties: false`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hooks {
    /// Runs on Pipeline creation.
    #[serde(default)]
    pub create: Option<Hook>,
    /// Runs every time the Pipeline state changes.
    #[serde(default)]
    pub change: Option<Hook>,
    /// Runs on Pipeline destruction.
    #[serde(default)]
    pub destroy: Option<Hook>,
    /// Runs to handle errors in the Pipeline.
    #[serde(default)]
    pub error: Option<Hook>,
}

/// The `hook` `$def`: an inline set of tasks, a reference to a Flow that implements the hook, or a
/// `{ use, inputs }` Flow import.
///
/// Untagged, tried in an order that resolves the object/object ambiguity: a string is a
/// [`Hook::Reference`]; an object bearing the required `use` key is a [`Hook::Use`] import; any
/// other object or array is an inline [`Hook::Tasks`] set. [`Hook::Use`] must precede
/// [`Hook::Tasks`] so a `{ use, inputs }` object is not misread as a one-key task map named `use`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Hook {
    /// A path/name reference to a Flow implementing the hook.
    Reference(String),
    /// A `{ use, inputs }` import of another Flow as the hook body.
    Use(HookUse),
    /// An inline set of tasks (array form or name-keyed map form).
    Tasks(crate::flow::Tasks),
}

/// The `{ use, inputs }` object form of a [`Hook`] — import another Flow as the hook body.
/// `additionalProperties: false`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HookUse {
    /// Path/name reference to the Flow to run as the hook.
    #[serde(rename = "use")]
    pub use_ref: String,
    /// Input variables passed into the referenced Flow, keyed by its declared input names.
    #[serde(default)]
    pub inputs: Option<Value>,
}
