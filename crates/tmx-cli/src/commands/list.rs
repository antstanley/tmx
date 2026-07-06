//! `tmx list` — discovery behind [`Discover`] (07 §`tmx list`).
//!
//! Enumerates one of four kinds as a JSON listing: `flows` (the conventional Flow files present in
//! the working directory plus any registered-name → path mappings from the resolved config),
//! `tasks` and `inputs` (projected from a resolved, preflighted Flow), and `providers` (the entries
//! of the project-local provider registry). `tasks`/`inputs` fail-fast on a malformed target Flow;
//! the directory-scoped kinds tolerate an empty project as an empty listing.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{Value, json};

use tmx_core::RunError;
use tmx_core::ports::driving::{Discover, DiscoverKind};

use crate::args::ListArgs;
use crate::commands::inspect::{resolve_and_preflight, task_type_name};
use crate::commands::provider::{read_registry_at, registry_path_for};
use crate::config::{self, EffectiveConfig};

/// The `Discover` use case over the working directory, resolved config, and provider registry.
pub struct EngineDiscover {
    /// The project directory the flow search and registry are anchored at.
    root: PathBuf,
    /// The resolved effective config (registered-name → path mappings feed `flows`).
    config: EffectiveConfig,
}

impl EngineDiscover {
    /// Wire discovery rooted at `root` with the resolved `config`.
    #[must_use]
    pub fn new(root: PathBuf, config: EffectiveConfig) -> Self {
        Self { root, config }
    }

    /// The `flows` listing: the conventional Flow files present in the root, plus registered names.
    fn flows(&self) -> Value {
        let mut flows: Vec<Value> = Vec::new();
        for candidate in config::flow_file_candidates() {
            let path = self.root.join(&candidate);
            if path.is_file() {
                flows.push(json!({ "name": candidate, "path": path.to_string_lossy() }));
            }
        }
        for (name, path) in &self.config.registered_names() {
            flows.push(json!({ "name": name, "path": path, "registered": true }));
        }
        json!({ "flows": flows })
    }

    /// The `providers` listing: the entries of the project-local registry.
    fn providers(&self) -> Result<Value, RunError> {
        let registry = read_registry_at(&registry_path_for(&self.root))?;
        let providers: Vec<Value> = registry
            .iter()
            .map(|(name, path)| json!({ "name": name, "path": path }))
            .collect();
        Ok(json!({ "providers": providers }))
    }
}

#[async_trait]
impl Discover for EngineDiscover {
    async fn discover(
        &self,
        kind: DiscoverKind,
        reference: Option<&str>,
    ) -> Result<Value, RunError> {
        match kind {
            DiscoverKind::Flows => Ok(self.flows()),
            DiscoverKind::Providers => self.providers(),
            DiscoverKind::Tasks => {
                let preflighted = resolve_and_preflight(reference, None).await?;
                let tasks: Vec<Value> = preflighted
                    .flow
                    .tasks
                    .iter()
                    .map(|task| {
                        json!({
                            "name": task.name.clone().unwrap_or_default(),
                            "type": task_type_name(&task.with),
                        })
                    })
                    .collect();
                Ok(json!({ "tasks": tasks }))
            }
            DiscoverKind::Inputs => {
                let preflighted = resolve_and_preflight(reference, None).await?;
                let inputs: Vec<Value> = preflighted
                    .flow
                    .inputs
                    .iter()
                    .map(|(name, spec)| {
                        json!({
                            "name": name,
                            "type": spec.input_type.clone(),
                            "required": spec.required.unwrap_or(false),
                        })
                    })
                    .collect();
                Ok(json!({ "inputs": inputs }))
            }
        }
    }
}

/// Run `tmx list <kind> [flow]`, returning the JSON listing.
///
/// # Errors
///
/// Returns `resolution`/`validation` when a `tasks`/`inputs` target Flow cannot be resolved or is
/// malformed; the `flows`/`providers` kinds tolerate an empty project.
pub async fn execute(args: ListArgs, profile: Option<String>) -> Result<Value, RunError> {
    let root = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    // The `--profile` global joins the flag layer, so a selected profile's overrides (e.g. its
    // registered-name → path mappings) take effect for `tmx list flows`.
    let mut flags = config::ConfigLayer::new();
    if let Some(profile) = profile {
        flags.insert("profile".to_string(), Value::String(profile));
    }
    let config = config::load_effective(flags, &root);
    let reference = args.file.clone().or_else(|| args.flow.clone());
    let use_case = EngineDiscover::new(root, config);
    use_case
        .discover(args.kind.to_kind(), reference.as_deref())
        .await
}
