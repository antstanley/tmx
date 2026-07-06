//! `tmx validate` — schema validation of one or more artifacts behind [`ValidateArtifacts`]
//! (07 §`tmx validate`).
//!
//! For each path: detect the wire format, load it into the shared model, `kind`-dispatch it to its
//! artifact class, and validate it against that class's schema. A **malformed** artifact — one that
//! does not parse — aborts fast with its typed error (`validation` → exit 3, or `resolution` → exit
//! 4 for a missing file) before any diagnostics are printed; a well-formed but schema-invalid
//! artifact contributes error [`Diagnostic`]s, and any error-severity finding makes the command exit
//! 3. When no path is given, the Flow is resolved by the same search order as `tmx run` and that one
//! Flow is validated.

use async_trait::async_trait;

use tmx_adapters::loader::{FileSourceLoader, classify_artifact, detect_source_kind};
use tmx_adapters::validate::JsonSchemaValidator;

use tmx_core::ports::driven::SourceLoader;
use tmx_core::ports::driving::ValidateArtifacts;
use tmx_core::{Diagnostic, RunError, Severity};

use crate::args::{RunArgs, ValidateArgs};
use crate::commands::run::resolve_target;
use crate::config;

/// The `ValidateArtifacts` use case over the built-in loader + schema validator.
pub struct EngineValidateArtifacts {
    loader: FileSourceLoader,
    schema: JsonSchemaValidator,
}

impl EngineValidateArtifacts {
    /// Wire the use case, compiling the embedded data-model schema.
    ///
    /// # Errors
    ///
    /// Returns the schema-compile error if the embedded schema is invalid.
    pub fn new() -> Result<Self, RunError> {
        Ok(Self {
            loader: FileSourceLoader::new(),
            schema: JsonSchemaValidator::new()?,
        })
    }
}

#[async_trait]
impl ValidateArtifacts for EngineValidateArtifacts {
    async fn validate(&self, paths: Vec<String>) -> Result<Vec<Diagnostic>, RunError> {
        let mut diagnostics = Vec::new();
        for path in &paths {
            // Fail-fast: a bad extension (`resolution`) or an unparseable document (`validation`) is a
            // typed error surfaced before any diagnostic is collected.
            let kind = detect_source_kind(path)?;
            let value = self.loader.load(path, kind).await?;
            let class = classify_artifact(path, &value)?;
            for diagnostic in self.schema.validate_class(&value, class) {
                // Annotate each finding with its source path so a multi-artifact run is traceable.
                diagnostics.push(diagnostic.with_path(path.clone()));
            }
        }
        Ok(diagnostics)
    }
}

/// The result of a `tmx validate`: every finding across the validated artifacts.
pub struct ValidateReport {
    /// The schema-validation findings, in discovery order.
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidateReport {
    /// Whether any finding is error-severity — the report is then blocking (exit 3).
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Error))
    }
}

/// Run `tmx validate` to its [`ValidateReport`] (or a typed [`RunError`] for a malformed / missing
/// artifact, fail-fast). When `args.paths` is empty, the Flow is resolved by the `tmx run` order and
/// validated as the single artifact.
///
/// # Errors
///
/// Returns `resolution` (a missing file / unresolved Flow) or `validation` (an unparseable artifact),
/// each surfaced before any finding is printed.
pub async fn execute(args: ValidateArgs) -> Result<ValidateReport, RunError> {
    let paths = if args.paths.is_empty() {
        vec![resolve_single_flow()?]
    } else {
        args.paths.clone()
    };
    let use_case = EngineValidateArtifacts::new()?;
    let diagnostics = use_case.validate(paths).await?;
    Ok(ValidateReport { diagnostics })
}

/// Resolve the Flow by the `tmx run` search order and return its single file reference — the artifact
/// `tmx validate` checks when no path is given. A directory layout is rejected: `validate` checks a
/// named artifact, not an assembled directory.
fn resolve_single_flow() -> Result<String, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let resolved = resolve_target(&RunArgs::default(), &cwd, config::env_flow())?;
    resolved.file_reference.ok_or_else(|| {
        RunError::resolution(
            "validate_requires_file",
            "tmx validate checks a named artifact file, not a directory layout",
        )
    })
}
