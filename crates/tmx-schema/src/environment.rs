//! `Environment` and `Resources` — the declarative description of where a Flow runs. Deserialise-
//! only mirror of the `environment` and `resources` `$def`s in
//! [`docs/tmx.schema.json`](../../../docs/tmx.schema.json).
//!
//! An Environment is consumed by an Environment Provider to provision a Pipeline's runtime. It is
//! an **open** object (`additionalProperties: true`): provider-specific keys at the top level and
//! under `options` are permitted, so the mirror captures the unknown keys in
//! [`Environment::extra`] rather than discarding them.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::Hook;

/// The `environment` `$def`: a declarative runtime description for a Flow.
///
/// Reusable and optionally standalone (`environment.*`). The named fields below mirror the schema's
/// documented keys; any other key (provider-specific) is preserved in [`Environment::extra`].
///
/// **Serialisation.** The mirror is `Serialize` as well as `Deserialize` so a resolved environment
/// can be handed to a `BinaryProvider` on stdin (06 §Environment and provider execution). The lone
/// exception is [`bootstrap`](Environment::bootstrap): serialising it would cascade a `Serialize`
/// bound through the whole task model, and a provider binary receives the *substrate* description
/// (image/resources/options), not the Flow's init tasks — so it is `skip_serializing`. Absent
/// optional fields are omitted rather than emitted as `null`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    /// Optional artifact discriminator; the constant `"environment"` for a standalone file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Human-readable name of the Environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Free-text description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Operating system, e.g. `linux`, `darwin`, `windows`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// CPU architecture, e.g. `amd64`, `arm64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    /// Target platform: local machine or a cloud provider (`local`, `aws`, `gcp`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Name of the Environment Provider that materialises this environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Execution substrate: `container`, `vm`, `microvm`, `cloud-instance`, or `process`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Standard image reference — a container image or a machine/VM image id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Requested resource allocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    /// Tasks to run on environment/container init, following the Flow task model. Not serialised
    /// (see the type docs): a `BinaryProvider` receives the substrate description, not the init tasks.
    #[serde(default, skip_serializing)]
    pub bootstrap: Option<Hook>,
    /// Provider/platform-specific options; free-form (`additionalProperties: true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    /// Any additional provider-specific top-level keys, preserved in source order.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// The `resources` `$def`: a requested resource allocation. `additionalProperties: false`.
///
/// `cpu` and `gpu` are `string | number` in the schema (e.g. `2` or `"500m"`), so they are modelled
/// as an untyped [`serde_json::Value`] to admit both without loss.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resources {
    /// vCPU count or quantity, e.g. `2` or `"500m"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Value>,
    /// Memory quantity, e.g. `"512Mi"`, `"2Gi"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// Storage quantity, e.g. `"10Gi"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    /// GPU count or type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<Value>,
}
