//! `tmx init` — scaffold a starter Flow behind [`ScaffoldFlow`] (07 §`tmx init`).
//!
//! Emits either a **single-file** Flow (`<name>.yaml`) or a **folder layout** (a `<name>/` directory
//! holding a shared `environment.yaml` and a starter task file that assemble into one Flow). Every
//! artifact it writes is built as the shared model and **schema-validated before it is written**, so
//! the scaffold it produces is guaranteed to load and validate — a fresh `tmx validate`/`tmx run`
//! on the output passes by construction. Returns the paths it created.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};

use tmx_adapters::loader::{ArtifactClass, emit_source};
use tmx_adapters::validate::JsonSchemaValidator;

use tmx_core::ports::driven::SourceKind;
use tmx_core::ports::driving::{ScaffoldFlow, ScaffoldLayout};
use tmx_core::{RunError, Severity};

use crate::args::InitArgs;

/// The default scaffold template name (the only one in v0).
const DEFAULT_TEMPLATE: &str = "basic";

/// The `ScaffoldFlow` use case, writing under a base directory and self-validating each artifact.
pub struct EngineScaffoldFlow {
    base_dir: PathBuf,
    schema: JsonSchemaValidator,
}

impl EngineScaffoldFlow {
    /// Wire the use case rooted at `base_dir`, compiling the embedded schema for self-validation.
    ///
    /// # Errors
    ///
    /// Returns the schema-compile error if the embedded schema is invalid.
    pub fn new(base_dir: PathBuf) -> Result<Self, RunError> {
        Ok(Self {
            base_dir,
            schema: JsonSchemaValidator::new()?,
        })
    }

    /// Build, validate, emit (as YAML), and write one artifact of `class` to `path`.
    fn write_artifact(
        &self,
        path: &Path,
        class: ArtifactClass,
        value: &Value,
    ) -> Result<(), RunError> {
        // Self-validate before writing: a scaffold that would not validate is a bug caught here, not a
        // broken starter shipped to the user.
        if let Some(error) = self
            .schema
            .validate_class(value, class)
            .into_iter()
            .find(|d| matches!(d.severity, Severity::Error))
        {
            return Err(RunError::validation(
                "scaffold_invalid",
                format!(
                    "the generated scaffold does not validate: {}",
                    error.message
                ),
            ));
        }
        let text = emit_source(value, SourceKind::Yaml)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                RunError::resolution(
                    "scaffold_dir_uncreatable",
                    format!("could not create `{}`: {e}", parent.display()),
                )
            })?;
        }
        std::fs::write(path, text).map_err(|e| {
            RunError::resolution(
                "scaffold_unwritable",
                format!("could not write scaffold `{}`: {e}", path.display()),
            )
        })
    }
}

#[async_trait]
impl ScaffoldFlow for EngineScaffoldFlow {
    async fn scaffold(
        &self,
        name: &str,
        _template: &str,
        layout: ScaffoldLayout,
    ) -> Result<Vec<String>, RunError> {
        match layout {
            ScaffoldLayout::SingleFile => {
                let path = self.base_dir.join(format!("{name}.yaml"));
                self.write_artifact(&path, ArtifactClass::Flow, &single_file_flow(name))?;
                Ok(vec![path_string(&path)])
            }
            ScaffoldLayout::Folder => {
                let dir = self.base_dir.join(name);
                let environment = dir.join("environment.yaml");
                let task = dir.join("01-greet.yaml");
                self.write_artifact(
                    &environment,
                    ArtifactClass::Environment,
                    &folder_environment(),
                )?;
                self.write_artifact(&task, ArtifactClass::Task, &folder_task(name))?;
                Ok(vec![path_string(&environment), path_string(&task)])
            }
        }
    }
}

/// The single-file starter Flow: one named `exec` task.
fn single_file_flow(name: &str) -> Value {
    json!({
        "name": name,
        "description": "A starter TMX Flow — edit the tasks below.",
        "tasks": [
            {
                "name": "greet",
                "type": "exec",
                "with": { "command": format!("echo \"hello from {name}\"") }
            }
        ]
    })
}

/// The folder layout's shared environment (a local substrate).
fn folder_environment() -> Value {
    json!({
        "kind": "environment",
        "platform": "local",
    })
}

/// The folder layout's starter task file (a standalone named `exec` task).
fn folder_task(name: &str) -> Value {
    json!({
        "kind": "task",
        "name": "greet",
        "type": "exec",
        "with": { "command": format!("echo \"hello from {name}\"") }
    })
}

/// Render a path as a string, falling back to a lossy form for a non-UTF-8 path.
fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Run `tmx init`, scaffolding a starter Flow and returning the paths it created.
///
/// # Errors
///
/// Returns `resolution` for an unwritable target, or `validation` if the generated scaffold would
/// not itself validate (a defensive guard on the templates).
pub async fn execute(args: InitArgs) -> Result<Vec<String>, RunError> {
    let base_dir = match &args.dir {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().map_err(|e| {
            RunError::resolution(
                "cwd_unavailable",
                format!("could not read the current working directory: {e}"),
            )
        })?,
    };
    let name = args.name.as_deref().unwrap_or("flow");
    let layout = if args.folder {
        ScaffoldLayout::Folder
    } else {
        ScaffoldLayout::SingleFile
    };
    let use_case = EngineScaffoldFlow::new(base_dir)?;
    use_case.scaffold(name, DEFAULT_TEMPLATE, layout).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmx_adapters::loader::{FileSourceLoader, classify_artifact, detect_source_kind};
    use tmx_core::ports::driven::SourceLoader;

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        use std::task::{Context, Poll};
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = std::pin::pin!(fut);
        loop {
            if let Poll::Ready(value) = fut.as_mut().poll(&mut cx) {
                return value;
            }
        }
    }

    /// Load and schema-validate the file the scaffold wrote, asserting it validates cleanly.
    fn assert_validates(path: &str) {
        let kind = detect_source_kind(path).expect("known extension");
        let loader = FileSourceLoader::new();
        let value = block_on(loader.load(path, kind)).expect("the scaffold loads");
        let class = classify_artifact(path, &value).expect("the scaffold classifies");
        let schema = JsonSchemaValidator::new().expect("schema compiles");
        let errors: Vec<_> = schema
            .validate_class(&value, class)
            .into_iter()
            .filter(|d| matches!(d.severity, Severity::Error))
            .collect();
        assert!(
            errors.is_empty(),
            "the scaffold `{path}` must validate, got {errors:?}"
        );
    }

    #[test]
    fn single_file_scaffold_validates() {
        let dir = std::env::temp_dir().join(format!("tmx-init-single-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let use_case = EngineScaffoldFlow::new(dir.clone()).expect("wire");
        let created =
            block_on(use_case.scaffold("demo", DEFAULT_TEMPLATE, ScaffoldLayout::SingleFile))
                .expect("the single-file scaffold writes");
        assert_eq!(created.len(), 1, "a single-file scaffold is one file");
        assert!(created[0].ends_with("demo.yaml"), "named after the flow");
        assert_validates(&created[0]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn folder_scaffold_writes_and_validates_each_artifact() {
        let dir = std::env::temp_dir().join(format!("tmx-init-folder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let use_case = EngineScaffoldFlow::new(dir.clone()).expect("wire");
        let created = block_on(use_case.scaffold("proj", DEFAULT_TEMPLATE, ScaffoldLayout::Folder))
            .expect("the folder scaffold writes");
        assert_eq!(
            created.len(),
            2,
            "a folder scaffold is environment + a task"
        );
        for path in &created {
            assert_validates(path);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
