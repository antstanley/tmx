//! The [`SourceLoader`] adapter: parse a YAML / JSON / JSONC / TOML source file into **one identical**
//! [`serde_json::Value`] model, and classify an artifact by its `kind` (or filename convention).
//!
//! This is TMX's defining trait made concrete: four wire formats, one model. Everything downstream of
//! the loader — validation, desugaring, the runner — is format-agnostic because all four front-ends
//! ([`json`], [`jsonc`], [`yaml`], [`toml`]) land in the same `Value`, and `serde_json`'s
//! `preserve_order` feature keeps a task *map* form in its source key order
//! ([`.specs/03-loading-and-preflight.md` §Source loading and `kind` dispatch](../../../../.specs/03-loading-and-preflight.md)).
//!
//! `kind` dispatch is layered: an explicit `kind` discriminator wins; absent that, the reserved
//! `environment.*` / `context.*` / `flow.*` filename convention decides; absent that, "a top-level
//! document with `tasks` is a Flow"; anything else is a task (a task file may use any filename).

pub mod emit;
pub(crate) mod json;
pub(crate) mod jsonc;
pub(crate) mod toml;
pub(crate) mod yaml;

pub use emit::emit_source;

use std::path::Path;

use async_trait::async_trait;
use serde_json::Value;

use tmx_core::error::RunError;
use tmx_core::ports::driven::{SourceKind, SourceLoader};

/// The artifact class a source document dispatches to — the `kind` column of the 03 dispatch table.
///
/// Distinct from [`SourceKind`] (the *wire format*): this is *what the document is* (a Flow, an
/// Environment, …), which selects the schema target the validator (task 14) checks it against. It
/// carries a `Task` variant the port-level [`ArtifactKind`](tmx_core::ports::driven::ArtifactKind)
/// omits, because a standalone task file is a valid loadable artifact even though it is validated as
/// part of a Flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactClass {
    /// A Flow (the default for a top-level document with `tasks`).
    Flow,
    /// A standalone Environment (`kind: environment` or the `environment.*` filename).
    Environment,
    /// A standalone Context (`kind: context` or the `context.*` filename).
    Context,
    /// A standalone task (`kind: task`, or any non-reserved file without top-level `tasks`).
    Task,
    /// A provider manifest (`kind: provider`).
    Provider,
}

/// The built-in file-backed [`SourceLoader`]: reads a path and parses it by its extension.
///
/// Stateless — the format is chosen per call from the `kind` argument the caller derived from the
/// extension via [`detect_source_kind`], so one instance serves every load.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileSourceLoader;

impl FileSourceLoader {
    /// A fresh loader.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SourceLoader for FileSourceLoader {
    async fn load(&self, path: &str, kind: SourceKind) -> Result<Value, RunError> {
        // Read is the effecting boundary; a missing/unreadable path is a typed resolution error
        // naming the path, never a panic. The read is synchronous std::fs (immediately ready), so
        // this adapter pulls in no async runtime — tokio arrives with the process/HTTP adapters.
        let text = std::fs::read_to_string(path).map_err(|e| {
            RunError::resolution(
                "source_unreadable",
                format!("could not read source `{path}`: {e}"),
            )
            .with_path(path.to_string())
        })?;
        parse_source(&text, kind).map_err(|e| e.with_path(path.to_string()))
    }
}

/// Parse `text` as `kind` into the shared [`Value`] model — the format-dispatch under [`load`].
///
/// [`load`](FileSourceLoader::load).
pub fn parse_source(text: &str, kind: SourceKind) -> Result<Value, RunError> {
    match kind {
        SourceKind::Json => json::parse(text),
        SourceKind::Jsonc => jsonc::parse(text),
        SourceKind::Yaml => yaml::parse(text),
        SourceKind::Toml => toml::parse(text),
    }
}

/// Select the [`SourceKind`] for `path` from its file extension.
///
/// `.yaml`/`.yml` → YAML, `.json` → JSON, `.jsonc` → JSONC, `.toml` → TOML. An unknown or missing
/// extension is a typed [`ErrorCategory::Resolution`](tmx_core::error::ErrorCategory::Resolution)
/// `RunError` (`code: unknown_source_format`) — never a guess.
pub fn detect_source_kind(path: &str) -> Result<SourceKind, RunError> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("yaml" | "yml") => Ok(SourceKind::Yaml),
        Some("json") => Ok(SourceKind::Json),
        Some("jsonc") => Ok(SourceKind::Jsonc),
        Some("toml") => Ok(SourceKind::Toml),
        _ => Err(RunError::resolution(
            "unknown_source_format",
            format!("unrecognised source extension for `{path}` (want yaml/yml/json/jsonc/toml)"),
        )
        .with_path(path.to_string())),
    }
}

/// Classify the loaded `value` at `path` into its [`ArtifactClass`] — the 03 `kind` dispatch.
///
/// Precedence: an explicit `kind` string wins; then the reserved `environment.*` / `context.*` /
/// `flow.*` filename convention; then "a top-level document with `tasks` is a Flow"; then a task. An
/// explicit but out-of-vocabulary `kind` is a typed
/// [`ErrorCategory::Resolution`](tmx_core::error::ErrorCategory::Resolution) `RunError`
/// (`code: unknown_artifact_kind`), not a silent fallthrough.
pub fn classify_artifact(path: &str, value: &Value) -> Result<ArtifactClass, RunError> {
    if let Some(kind) = value.get("kind").and_then(Value::as_str) {
        return match kind {
            "flow" => Ok(ArtifactClass::Flow),
            "environment" => Ok(ArtifactClass::Environment),
            "context" => Ok(ArtifactClass::Context),
            "task" => Ok(ArtifactClass::Task),
            "provider" => Ok(ArtifactClass::Provider),
            other => Err(RunError::resolution(
                "unknown_artifact_kind",
                format!("unknown artifact kind `{other}` in `{path}`"),
            )
            .with_path(path.to_string())),
        };
    }
    match reserved_stem(path) {
        Some("environment") => return Ok(ArtifactClass::Environment),
        Some("context") => return Ok(ArtifactClass::Context),
        Some("flow") => return Ok(ArtifactClass::Flow),
        _ => {}
    }
    if value.get("tasks").is_some() {
        return Ok(ArtifactClass::Flow);
    }
    Ok(ArtifactClass::Task)
}

/// The reserved filename stem of `path` — the segment before the first `.` of the basename, when it
/// is exactly one of the reserved names; otherwise `None`.
///
/// `environment.toml` → `Some("environment")`, `task-1.jsonc` → `None`. Only the exact reserved names
/// are special; `environment-prod.yaml` is a task, not the shared environment.
fn reserved_stem(path: &str) -> Option<&str> {
    let name = Path::new(path).file_name().and_then(|n| n.to_str())?;
    let stem = name.split_once('.').map_or(name, |(head, _)| head);
    match stem {
        "environment" | "context" | "flow" => Some(stem),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_source_kind_maps_each_extension_and_rejects_unknown() {
        assert_eq!(
            detect_source_kind("a/b/flow.yaml").expect("yaml"),
            SourceKind::Yaml
        );
        assert_eq!(detect_source_kind("x.yml").expect("yml"), SourceKind::Yaml);
        assert_eq!(
            detect_source_kind("x.json").expect("json"),
            SourceKind::Json
        );
        assert_eq!(
            detect_source_kind("x.jsonc").expect("jsonc"),
            SourceKind::Jsonc
        );
        assert_eq!(
            detect_source_kind("x.toml").expect("toml"),
            SourceKind::Toml
        );
        // Negative space: an unknown extension and a bare (extension-less) name are both typed
        // errors, not a defaulted-to-YAML guess.
        let unknown = detect_source_kind("notes.txt").expect_err("unknown extension");
        assert_eq!(unknown.code, "unknown_source_format", "typed format error");
        assert_eq!(unknown.path.as_deref(), Some("notes.txt"), "names the path");
        assert!(
            detect_source_kind("README").is_err(),
            "a missing extension is not a silent default"
        );
    }

    #[test]
    fn classify_prefers_explicit_kind_over_everything() {
        // An explicit `kind` wins even when the filename or `tasks` would say otherwise.
        let env_named_like_a_flow = json!({ "kind": "environment", "tasks": [] });
        assert_eq!(
            classify_artifact("flow.yaml", &env_named_like_a_flow).expect("classifies"),
            ArtifactClass::Environment,
            "explicit kind overrides the flow.* filename and the presence of tasks"
        );
        assert_eq!(
            classify_artifact("m.yaml", &json!({ "kind": "provider" })).expect("provider"),
            ArtifactClass::Provider
        );
    }

    #[test]
    fn classify_falls_back_to_filename_then_tasks_then_task() {
        // No explicit kind: the reserved filename decides first.
        assert_eq!(
            classify_artifact("dir/environment.toml", &json!({ "platform": "aws" }))
                .expect("env by name"),
            ArtifactClass::Environment
        );
        assert_eq!(
            classify_artifact("dir/context.yaml", &json!({ "env": {} })).expect("ctx by name"),
            ArtifactClass::Context
        );
        // A non-reserved filename with top-level `tasks` is a Flow…
        assert_eq!(
            classify_artifact("pipeline.yaml", &json!({ "tasks": [] })).expect("flow by tasks"),
            ArtifactClass::Flow
        );
        // …and any other non-reserved file is a task (identity is "not a reserved artifact").
        assert_eq!(
            classify_artifact("task-1.jsonc", &json!({ "type": "exec", "with": {} }))
                .expect("task"),
            ArtifactClass::Task
        );
        // A near-miss reserved name is a task, not the shared environment.
        assert_eq!(
            classify_artifact("environment-prod.yaml", &json!({ "type": "exec" }))
                .expect("near-miss"),
            ArtifactClass::Task,
            "only the exact reserved stem is special"
        );
    }

    #[test]
    fn classify_rejects_an_unknown_explicit_kind() {
        // Negative space: an explicit but out-of-vocabulary kind is a typed error, not a fallthrough.
        let err = classify_artifact("x.yaml", &json!({ "kind": "teleport" }))
            .expect_err("unknown kind is rejected");
        assert_eq!(err.code, "unknown_artifact_kind", "typed dispatch error");
        assert_eq!(
            err.category,
            tmx_core::error::ErrorCategory::Resolution,
            "an unknown kind is a resolution-category error"
        );
    }

    #[test]
    fn parse_source_reaches_every_front_end() {
        // Each wire format lands in the same Value for the same logical document.
        let want = json!({ "a": 1, "b": "two" });
        assert_eq!(
            parse_source(r#"{ "a": 1, "b": "two" }"#, SourceKind::Json).expect("json"),
            want
        );
        assert_eq!(
            parse_source("{ \"a\": 1, /* c */ \"b\": \"two\" }", SourceKind::Jsonc).expect("jsonc"),
            want
        );
        assert_eq!(
            parse_source("a: 1\nb: two\n", SourceKind::Yaml).expect("yaml"),
            want
        );
        assert_eq!(
            parse_source("a = 1\nb = \"two\"\n", SourceKind::Toml).expect("toml"),
            want
        );
    }
}
