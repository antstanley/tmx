//! `tmx provider` — the provider-registry commands behind [`ManageProviders`] (07 §`tmx provider`;
//! 06 §Cross-cutting driven ports `ManageProviders`).
//!
//! Reads and writes a small project-local registry (`./.tmx/providers.json`) of registered-name →
//! manifest-path mappings, and validates a manifest against the provider schema through the same
//! [`load_manifest`](tmx_adapters::provider::load_manifest) path the `tmx env` lifecycle uses. A
//! `register` **validates the manifest before recording it**, so a malformed manifest is rejected
//! (exit 3) and never enters the registry; `validate` checks a manifest without recording it. `show`
//! returns a registered manifest's JSON; `list` enumerates the registry.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use tmx_adapters::provider::load_manifest;
use tmx_adapters::sink::Format;

use tmx_core::RunError;
use tmx_core::ports::driving::{ManageProviders, ProviderOp};

use crate::args::{ProviderArgs, ProviderCommand};
use crate::compose::Composed;

/// The registry file, relative to the project root (`./.tmx/providers.json`).
const REGISTRY_RELATIVE: &str = ".tmx/providers.json";

/// The `ManageProviders` use case over the project-local registry and the manifest loader.
pub struct EngineManageProviders {
    /// The project root the registry and relative manifest paths are anchored at.
    root: PathBuf,
    /// The composition rooted at `root`, lending the resolve/load/validate ports to `load_manifest`.
    composed: Composed,
}

impl EngineManageProviders {
    /// Wire the use case rooted at `root` (the project directory).
    ///
    /// # Errors
    ///
    /// Returns the embedded-schema compile error if the validator cannot be built.
    pub fn new(root: PathBuf) -> Result<Self, RunError> {
        let composed = Composed::new(root.clone(), Format::Json, false, None)?;
        Ok(Self { root, composed })
    }

    /// The registry file path under the project root.
    fn registry_path(&self) -> PathBuf {
        self.root.join(REGISTRY_RELATIVE)
    }

    /// Read the registry (name → manifest-path), tolerating an absent file as an empty registry. A
    /// malformed registry file is a typed `validation` error rather than a silent empty read.
    fn read_registry(&self) -> Result<Map<String, Value>, RunError> {
        read_registry_at(&self.registry_path())
    }

    /// Persist the registry, creating `./.tmx/` as needed.
    fn write_registry(&self, registry: &Map<String, Value>) -> Result<(), RunError> {
        let path = self.registry_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RunError::resolution(
                    "registry_dir_uncreatable",
                    format!("could not create `{}`: {e}", parent.display()),
                )
            })?;
        }
        let text = serde_json::to_string_pretty(&Value::Object(registry.clone())).map_err(|e| {
            RunError::validation(
                "registry_unrenderable",
                format!("could not render registry: {e}"),
            )
        })?;
        std::fs::write(&path, text).map_err(|e| {
            RunError::resolution(
                "registry_unwritable",
                format!("could not write registry `{}`: {e}", path.display()),
            )
        })
    }
}

#[async_trait]
impl ManageProviders for EngineManageProviders {
    async fn manage(&self, op: ProviderOp) -> Result<Value, RunError> {
        match op {
            ProviderOp::List => {
                let registry = self.read_registry()?;
                let providers: Vec<Value> = registry
                    .iter()
                    .map(|(name, path)| json!({ "name": name, "path": path }))
                    .collect();
                Ok(json!({ "providers": providers }))
            }
            ProviderOp::Show(name) => {
                let registry = self.read_registry()?;
                let path = registry.get(&name).and_then(Value::as_str).ok_or_else(|| {
                    RunError::resolution(
                        "provider_not_registered",
                        format!("no provider `{name}` is registered"),
                    )
                })?;
                let loaded = load_manifest(
                    path,
                    self.composed.preflight_ports().reference_resolver,
                    self.composed.preflight_ports().source_loader,
                    self.composed.preflight_ports().schema,
                )
                .await?;
                Ok(json!({ "name": name, "path": path, "manifest": loaded.raw }))
            }
            ProviderOp::Register(path) => {
                // Validate the manifest before recording it — a malformed manifest never enters the
                // registry (it is a fail-fast `validation` error, exit 3).
                let loaded = load_manifest(
                    &path,
                    self.composed.preflight_ports().reference_resolver,
                    self.composed.preflight_ports().source_loader,
                    self.composed.preflight_ports().schema,
                )
                .await?;
                let name = loaded.manifest.name.clone();
                let mut registry = self.read_registry()?;
                registry.insert(name.clone(), Value::String(path.clone()));
                self.write_registry(&registry)?;
                Ok(json!({ "registered": name, "path": path }))
            }
            ProviderOp::Remove(name) => {
                let mut registry = self.read_registry()?;
                if registry.remove(&name).is_none() {
                    return Err(RunError::resolution(
                        "provider_not_registered",
                        format!("no provider `{name}` is registered"),
                    ));
                }
                self.write_registry(&registry)?;
                Ok(json!({ "removed": name }))
            }
            ProviderOp::Validate(path) => {
                let loaded = load_manifest(
                    &path,
                    self.composed.preflight_ports().reference_resolver,
                    self.composed.preflight_ports().source_loader,
                    self.composed.preflight_ports().schema,
                )
                .await?;
                Ok(json!({ "valid": true, "name": loaded.manifest.name, "path": path }))
            }
        }
    }
}

/// Read the registry at `path` (name → manifest-path), tolerating an absent file as empty and
/// rejecting a malformed one as a typed `validation` error. Shared with `tmx list providers`.
pub fn read_registry_at(path: &Path) -> Result<Map<String, Value>, RunError> {
    if !path.is_file() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| {
        RunError::resolution(
            "registry_unreadable",
            format!("could not read registry `{}`: {e}", path.display()),
        )
    })?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(RunError::validation(
            "registry_not_object",
            format!(
                "registry `{}` must be a JSON object of name → path",
                path.display()
            ),
        )),
        Err(e) => Err(RunError::validation(
            "registry_invalid",
            format!("registry `{}` is not valid JSON: {e}", path.display()),
        )),
    }
}

/// The registry path under a project `root` — `./.tmx/providers.json`. Shared with `tmx list`.
#[must_use]
pub fn registry_path_for(root: &Path) -> PathBuf {
    root.join(REGISTRY_RELATIVE)
}

/// Map a parsed [`ProviderCommand`] to its use-case [`ProviderOp`].
fn to_op(command: ProviderCommand) -> ProviderOp {
    match command {
        ProviderCommand::List => ProviderOp::List,
        ProviderCommand::Show { name } => ProviderOp::Show(name),
        ProviderCommand::Register { path } => ProviderOp::Register(path),
        ProviderCommand::Remove { name } => ProviderOp::Remove(name),
        ProviderCommand::Validate { path } => ProviderOp::Validate(path),
    }
}

/// Run `tmx provider <sub>`, returning its JSON result.
///
/// # Errors
///
/// Returns `resolution` for an unregistered/unresolved provider or `validation` (exit 3) for a
/// malformed manifest or registry.
pub async fn execute(args: ProviderArgs) -> Result<Value, RunError> {
    let root = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let use_case = EngineManageProviders::new(root)?;
    use_case.manage(to_op(args.command)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_op_maps_each_subcommand() {
        assert_eq!(to_op(ProviderCommand::List), ProviderOp::List);
        assert_eq!(
            to_op(ProviderCommand::Show { name: "aws".into() }),
            ProviderOp::Show("aws".into())
        );
        assert_eq!(
            to_op(ProviderCommand::Register {
                path: "m.yaml".into()
            }),
            ProviderOp::Register("m.yaml".into())
        );
        assert_eq!(
            to_op(ProviderCommand::Remove { name: "aws".into() }),
            ProviderOp::Remove("aws".into())
        );
        assert_eq!(
            to_op(ProviderCommand::Validate {
                path: "m.yaml".into()
            }),
            ProviderOp::Validate("m.yaml".into())
        );
    }

    #[test]
    fn read_registry_tolerates_absent_and_rejects_malformed() {
        let dir = std::env::temp_dir().join(format!("tmx-prov-reg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        // Absent registry → empty.
        let absent = dir.join("nope.json");
        assert!(
            read_registry_at(&absent).expect("absent is Ok").is_empty(),
            "an absent registry reads as empty"
        );

        // Malformed registry → typed validation error (negative space).
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "[1,2,3]").expect("write");
        let err = read_registry_at(&bad).expect_err("a non-object registry is rejected");
        assert_eq!(err.code, "registry_not_object", "typed registry error");

        // A well-formed registry reads its entries.
        let good = dir.join("good.json");
        std::fs::write(&good, "{ \"aws\": \"providers/aws.yaml\" }").expect("write");
        let registry = read_registry_at(&good).expect("a good registry reads");
        assert_eq!(
            registry.get("aws").and_then(Value::as_str),
            Some("providers/aws.yaml"),
            "the registered path reads back"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
