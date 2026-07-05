//! `Environment` and `Resources` — the declarative description of where a Flow runs. Deserialise-
//! only mirror of the `environment` and `resources` `$def`s in
//! [`docs/tmx.schema.json`](../../../docs/tmx.schema.json).
//!
//! An Environment is consumed by an Environment Provider to provision a Pipeline's runtime. It is
//! an **open** object (`additionalProperties: true`): provider-specific keys at the top level and
//! under `options` are permitted, so the mirror captures the unknown keys in
//! [`Environment::extra`] rather than discarding them.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::context::Hook;

/// The `environment` `$def`: a declarative runtime description for a Flow.
///
/// Reusable and optionally standalone (`environment.*`). The named fields below mirror the schema's
/// documented keys; any other key (provider-specific) is preserved in [`Environment::extra`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    /// Optional artifact discriminator; the constant `"environment"` for a standalone file.
    #[serde(default)]
    pub kind: Option<String>,
    /// Human-readable name of the Environment.
    #[serde(default)]
    pub name: Option<String>,
    /// Free-text description.
    #[serde(default)]
    pub description: Option<String>,
    /// Operating system, e.g. `linux`, `darwin`, `windows`.
    #[serde(default)]
    pub os: Option<String>,
    /// CPU architecture, e.g. `amd64`, `arm64`.
    #[serde(default)]
    pub arch: Option<String>,
    /// Target platform: local machine or a cloud provider (`local`, `aws`, `gcp`, …).
    #[serde(default)]
    pub platform: Option<String>,
    /// Name of the Environment Provider that materialises this environment.
    #[serde(default)]
    pub provider: Option<String>,
    /// Execution substrate: `container`, `vm`, `microvm`, `cloud-instance`, or `process`.
    #[serde(default)]
    pub runtime: Option<String>,
    /// Standard image reference — a container image or a machine/VM image id.
    #[serde(default)]
    pub image: Option<String>,
    /// Requested resource allocation.
    #[serde(default)]
    pub resources: Option<Resources>,
    /// Tasks to run on environment/container init, following the Flow task model.
    #[serde(default)]
    pub bootstrap: Option<Hook>,
    /// Provider/platform-specific options; free-form (`additionalProperties: true`).
    #[serde(default)]
    pub options: Option<Value>,
    /// Any additional provider-specific top-level keys, preserved in source order.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// The `resources` `$def`: a requested resource allocation. `additionalProperties: false`.
///
/// `cpu` and `gpu` are `string | number` in the schema (e.g. `2` or `"500m"`), so they are modelled
/// as an untyped [`serde_json::Value`] to admit both without loss.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resources {
    /// vCPU count or quantity, e.g. `2` or `"500m"`.
    #[serde(default)]
    pub cpu: Option<Value>,
    /// Memory quantity, e.g. `"512Mi"`, `"2Gi"`.
    #[serde(default)]
    pub memory: Option<String>,
    /// Storage quantity, e.g. `"10Gi"`.
    #[serde(default)]
    pub storage: Option<String>,
    /// GPU count or type.
    #[serde(default)]
    pub gpu: Option<Value>,
}
