//! The [`SchemaValidator`] adapter: validate an artifact (or a task `produces` output) against the
//! TMX data-model schema, JSON Schema **Draft 2020-12**, `kind`-dispatched.
//!
//! This is the in-process port of [`scripts/validate.sh`](../../../../scripts/validate.sh): the same
//! two schema documents ([`docs/tmx.schema.json`](../../../../docs/tmx.schema.json) and
//! [`docs/tmx-provider.schema.json`](../../../../docs/tmx-provider.schema.json)), the same `kind`
//! dispatch (Flow / Environment / Context / Task / provider), and — by construction — the same
//! accept/reject verdict over the example corpus
//! ([`.specs/03-loading-and-preflight.md` §Validation](../../../../.specs/03-loading-and-preflight.md)).
//!
//! Both schemas are **embedded at compile time** via [`include_str!`], so the validator is hermetic:
//! it needs no repository path at run time and never reaches the network. The provider manifest's
//! method bodies carry a *cross-file* `$ref` into `tmx.schema.json` (`#/$defs/task`); that reference
//! is resolved by [`MainSchemaRetriever`], which hands the embedded main schema to the compiler
//! rather than fetching it — the reason the adapter can validate a provider offline.
//!
//! **Sync**: validation is pure CPU over an in-memory [`Value`] with no effecting boundary, mirroring
//! the [`SchemaValidator`] port. A schema violation is a [`Diagnostic`] (the artifact is *rejected*);
//! only an internal fault — a schema that fails to compile — is a typed [`RunError`], never a panic.

use serde_json::{Value, json};

use jsonschema::{Draft, Retrieve, Uri, Validator};

use tmx_core::error::RunError;
use tmx_core::model::{Diagnostic, Severity};
use tmx_core::ports::driven::{ArtifactKind, SchemaValidator};

use crate::loader::ArtifactClass;

/// The TMX data-model schema (Draft 2020-12), embedded from `docs/tmx.schema.json`. Its root `$ref`s
/// `#/$defs/flow`, and its `$defs` hold every artifact sub-schema the dispatch table names.
const MAIN_SCHEMA_JSON: &str = include_str!("../../../docs/tmx.schema.json");

/// The provider-manifest schema, embedded from `docs/tmx-provider.schema.json`. Its method bodies
/// `$ref` [`MAIN_SCHEMA_JSON`] cross-file for the task definition — resolved by [`MainSchemaRetriever`].
const PROVIDER_SCHEMA_JSON: &str = include_str!("../../../docs/tmx-provider.schema.json");

/// The stable diagnostic `code` every schema violation carries. Parity with `scripts/validate.sh` is
/// verdict-level (accept/reject), so one code suffices — the human `message` and `path` locate the
/// specific keyword failure, and the machine code stays a single closed value.
const SCHEMA_VIOLATION_CODE: &str = "schema_violation";

/// The JSON pointer reported for a violation at the document root (jsonschema yields the empty string
/// there; a `Diagnostic` path is more legible as the root pointer).
const ROOT_POINTER: &str = "/";

/// Resolves the provider manifest's cross-file `$ref` into the embedded main schema.
///
/// The provider schema references `https://tmx.dev/schemas/0.2.0/tmx.schema.json#/$defs/task`; when
/// the compiler needs that document it calls [`retrieve`](Retrieve::retrieve) with the base URI. This
/// retriever returns the embedded main schema for exactly that `$id` and errors for anything else —
/// so an *unexpected* external reference fails loudly at compile time instead of silently reaching
/// for the network (the HTTP retriever is not even built in — `default-features = false`).
#[derive(Debug, Clone)]
struct MainSchemaRetriever {
    /// The `$id` of the main schema — the only external document the provider manifest may reference.
    main_id: String,
    /// The parsed main schema handed back for that `$id`.
    main: Value,
}

impl Retrieve for MainSchemaRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if uri.as_str() == self.main_id {
            Ok(self.main.clone())
        } else {
            Err(format!(
                "no embedded schema for external reference `{}` (only `{}` is known)",
                uri.as_str(),
                self.main_id
            )
            .into())
        }
    }
}

/// The built-in JSON-Schema-2020-12 [`SchemaValidator`].
///
/// Holds one compiled [`Validator`] per artifact class. Compilation happens once in [`new`](Self::new);
/// every later `validate` call is a borrow-and-check with no reparsing. The five validators cover the
/// full `kind` dispatch — including [`ArtifactClass::Task`], which the port-level [`ArtifactKind`]
/// omits (a standalone task is validated through [`validate_class`](Self::validate_class), used by
/// preflight and the corpus parity test).
#[derive(Debug)]
pub struct JsonSchemaValidator {
    /// Validates a Flow (the whole main schema, whose root `$ref`s `#/$defs/flow`).
    flow: Validator,
    /// Validates a standalone Environment against `#/$defs/environment`.
    environment: Validator,
    /// Validates a standalone Context against `#/$defs/context`.
    context: Validator,
    /// Validates a standalone task against `#/$defs/task`.
    task: Validator,
    /// Validates a provider manifest (with the cross-file `$ref` resolved).
    provider: Validator,
}

impl JsonSchemaValidator {
    /// Compile the five artifact validators from the embedded schemas.
    ///
    /// # Errors
    ///
    /// Returns an [`ErrorCategory::Validation`](tmx_core::error::ErrorCategory::Validation)
    /// [`RunError`] if either embedded schema fails to parse or to compile — an internal fault (a
    /// broken schema file), distinct from a *rejected artifact*, which is a [`Diagnostic`].
    pub fn new() -> Result<Self, RunError> {
        let main: Value = serde_json::from_str(MAIN_SCHEMA_JSON).map_err(|e| {
            RunError::validation(
                "schema_unparsable",
                format!("embedded main schema is not valid JSON: {e}"),
            )
        })?;
        let provider_schema: Value = serde_json::from_str(PROVIDER_SCHEMA_JSON).map_err(|e| {
            RunError::validation(
                "schema_unparsable",
                format!("embedded provider schema is not valid JSON: {e}"),
            )
        })?;

        let main_id = main
            .get("$id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RunError::validation(
                    "schema_missing_id",
                    "embedded main schema has no `$id` for cross-file reference resolution",
                )
            })?
            .to_string();

        // Precondition for the def-wrappers: the main schema exposes its `$defs`.
        let defs = main.get("$defs").cloned().ok_or_else(|| {
            RunError::validation(
                "schema_missing_defs",
                "embedded main schema has no `$defs` object to dispatch kinds against",
            )
        })?;

        let retriever = MainSchemaRetriever {
            main_id,
            main: main.clone(),
        };

        // Flow validates against the whole main document (its root `$ref`s `#/$defs/flow`); the four
        // standalone kinds validate against a `$ref`-into-`$defs` wrapper, so a non-object $def is
        // not also forced through the root's `type: object` (the same shape scripts/validate.sh uses).
        let flow = compile(&main, &retriever)?;
        let environment = compile(&def_wrapper(&defs, "environment"), &retriever)?;
        let context = compile(&def_wrapper(&defs, "context"), &retriever)?;
        let task = compile(&def_wrapper(&defs, "task"), &retriever)?;
        let provider = compile(&provider_schema, &retriever)?;

        Ok(Self {
            flow,
            environment,
            context,
            task,
            provider,
        })
    }

    /// Validate `instance` as an artifact of `class`, returning any [`Diagnostic`]s (empty ⇒ valid).
    ///
    /// This is the full-dispatch entry point — it covers [`ArtifactClass::Task`], which the port
    /// [`SchemaValidator::validate`] cannot reach (a standalone task is validated here, by preflight
    /// and by the corpus parity test).
    #[must_use]
    pub fn validate_class(&self, instance: &Value, class: ArtifactClass) -> Vec<Diagnostic> {
        let validator = match class {
            ArtifactClass::Flow => &self.flow,
            ArtifactClass::Environment => &self.environment,
            ArtifactClass::Context => &self.context,
            ArtifactClass::Task => &self.task,
            ArtifactClass::Provider => &self.provider,
        };
        diagnostics_from(validator, instance)
    }
}

/// Wrap a single `$def` in a fresh 2020-12 schema whose sole keyword is a `$ref` into the lifted
/// `$defs`. Keeps a non-object `$def` from being forced through the root's `type: object`.
fn def_wrapper(defs: &Value, name: &str) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": defs,
        "$ref": format!("#/$defs/{name}"),
    })
}

/// Compile `schema` as Draft 2020-12 with the cross-file retriever wired in.
fn compile(schema: &Value, retriever: &MainSchemaRetriever) -> Result<Validator, RunError> {
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        // `format` is annotation-only in 2020-12; an unknown format must not fail compilation or
        // validation (parity with the Python validator, which likewise does not assert formats).
        .should_ignore_unknown_formats(true)
        .with_retriever(retriever.clone())
        .build(schema)
        .map_err(|e| {
            RunError::validation(
                "schema_uncompilable",
                format!("could not compile embedded schema: {e}"),
            )
        })
}

/// Collect every schema violation of `instance` under `validator` into a [`Diagnostic`], each naming
/// the failing JSON path. An empty vector means the instance is valid.
fn diagnostics_from(validator: &Validator, instance: &Value) -> Vec<Diagnostic> {
    validator
        .iter_errors(instance)
        .map(|error| {
            let pointer = error.instance_path().to_string();
            let path = if pointer.is_empty() {
                ROOT_POINTER.to_string()
            } else {
                pointer
            };
            Diagnostic::new(Severity::Error, SCHEMA_VIOLATION_CODE, error.to_string())
                .with_path(path)
        })
        .collect()
}

impl SchemaValidator for JsonSchemaValidator {
    fn validate(&self, instance: &Value, kind: ArtifactKind) -> Result<Vec<Diagnostic>, RunError> {
        // The port vocabulary is the four standalone/composite classes; a task is validated inside a
        // Flow (or via `validate_class` in preflight), so `ArtifactKind` carries no `Task` variant.
        let class = match kind {
            ArtifactKind::Flow => ArtifactClass::Flow,
            ArtifactKind::Environment => ArtifactClass::Environment,
            ArtifactKind::Context => ArtifactClass::Context,
            ArtifactKind::Provider => ArtifactClass::Provider,
        };
        Ok(self.validate_class(instance, class))
    }

    fn validate_produces(
        &self,
        output: &Value,
        schema: &Value,
    ) -> Result<Vec<Diagnostic>, RunError> {
        // A `produces` schema is user-supplied JSON Schema compiled on demand; an uncompilable one is
        // an internal `Validation` RunError (naming why), never a panic. Forcing the 2020-12 draft
        // keeps `produces` checking on the same dialect as artifact validation even when the fragment
        // omits `$schema`.
        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .should_ignore_unknown_formats(true)
            .build(schema)
            .map_err(|e| {
                RunError::validation(
                    "produces_schema_uncompilable",
                    format!("`produces` schema does not compile: {e}"),
                )
            })?;
        Ok(diagnostics_from(&validator, output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{classify_artifact, detect_source_kind, parse_source};
    use std::path::{Path, PathBuf};

    fn validator() -> JsonSchemaValidator {
        JsonSchemaValidator::new().expect("embedded schemas compile")
    }

    fn examples_dir() -> PathBuf {
        // CARGO_MANIFEST_DIR is crates/tmx-adapters; the corpus is two levels up under docs/.
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples")
    }

    /// Recursively collect every example file with a recognised source extension.
    fn corpus_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(dir).expect("read examples dir");
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                out.extend(corpus_files(&path));
            } else if detect_source_kind(&path.to_string_lossy()).is_ok() {
                out.push(path);
            }
        }
        out.sort();
        out
    }

    /// Load and classify one corpus file into `(class, value)` exactly as preflight would.
    fn load_and_classify(path: &Path) -> (ArtifactClass, Value) {
        let path_str = path.to_string_lossy().to_string();
        let kind = detect_source_kind(&path_str).expect("known extension");
        let text = std::fs::read_to_string(path).expect("read corpus file");
        let value = parse_source(&text, kind).expect("corpus file parses");
        let class = classify_artifact(&path_str, &value).expect("corpus file classifies");
        (class, value)
    }

    #[test]
    fn every_corpus_artifact_validates_at_parity_with_validate_sh() {
        // scripts/validate.sh accepts the entire example corpus; the in-process validator must reach
        // the identical accept verdict for every file, dispatched by its `kind`. Two assertions: a
        // healthy, non-vacuous file count, and zero diagnostics across the corpus.
        let validator = validator();
        let files = corpus_files(&examples_dir());
        assert!(
            files.len() >= 20,
            "the example corpus should be substantial (found {}), so this parity check is not vacuous",
            files.len()
        );

        let mut checked = 0_usize;
        for path in &files {
            let (class, value) = load_and_classify(path);
            let diagnostics = validator.validate_class(&value, class);
            assert!(
                diagnostics.is_empty(),
                "valid corpus artifact {} (class {class:?}) must accept, got: {diagnostics:?}",
                path.display()
            );
            checked += 1;
        }
        assert_eq!(
            checked,
            files.len(),
            "every discovered corpus file must be dispatched and checked"
        );
    }

    #[test]
    fn each_kind_dispatches_to_its_own_schema_not_a_catch_all() {
        // Proving dispatch selects the per-kind schema rather than one permissive catch-all: a Flow
        // object (with `tasks`) is rejected by the Context schema (whose `additionalProperties: false`
        // forbids `tasks`), and an environment-shaped object is rejected by the Flow schema (which
        // requires `tasks`). The `$defs/environment` schema is deliberately open, so it is not a
        // useful discriminator — Context and Flow are.
        let validator = validator();
        let flow = json!({ "tasks": [ { "name": "hello", "type": "exec", "with": { "command": "echo hi" } } ] });
        let environment = json!({ "name": "dev", "provider": "local-docker" });

        assert!(
            validator
                .validate_class(&flow, ArtifactClass::Flow)
                .is_empty(),
            "a well-formed flow validates as Flow"
        );
        assert!(
            !validator
                .validate_class(&flow, ArtifactClass::Context)
                .is_empty(),
            "a flow is NOT a valid Context — dispatch must not fall through to a catch-all"
        );
        assert!(
            !validator
                .validate_class(&environment, ArtifactClass::Flow)
                .is_empty(),
            "an environment is NOT a valid Flow (no `tasks`)"
        );
    }

    #[test]
    fn a_standalone_task_validates_against_the_task_schema() {
        let validator = validator();
        let ok = json!({ "type": "exec", "with": { "command": "echo hi" } });
        let bad = json!({ "type": "totally-not-a-real-task-type" });

        assert!(
            validator
                .validate_class(&ok, ArtifactClass::Task)
                .is_empty(),
            "a minimal exec task is valid against #/$defs/task"
        );
        let diagnostics = validator.validate_class(&bad, ArtifactClass::Task);
        assert!(
            !diagnostics.is_empty(),
            "an unknown task `type` must be rejected"
        );
        assert!(
            diagnostics.iter().all(|d| d.severity == Severity::Error),
            "a schema rejection is an error-severity diagnostic"
        );
    }

    #[test]
    fn cross_file_ref_into_the_provider_manifest_resolves() {
        // The provider manifest's method bodies $ref tmx.schema.json#/$defs/task. If the cross-file
        // reference did not resolve, the provider validator would either fail to compile or accept a
        // structurally-broken task body. We prove both directions: the real corpus manifest (valid
        // task bodies) accepts, and a manifest with a bogus task body inside a method is rejected.
        let validator = validator();

        let (class, manifest) = load_and_classify(&examples_dir().join("provider-manifest.yaml"));
        assert_eq!(
            class,
            ArtifactClass::Provider,
            "the provider manifest classifies as a provider artifact"
        );
        assert!(
            validator
                .validate_class(&manifest, ArtifactClass::Provider)
                .is_empty(),
            "the real provider manifest validates — its cross-file task $ref resolved and accepted"
        );

        // A method body whose inline task carries an unknown `type` violates the cross-file task
        // schema; a resolved $ref must catch it.
        let broken = json!({
            "kind": "provider",
            "name": "broken",
            "type": "flow",
            "methods": {
                "bootstrap": [ { "type": "no-such-task-type" } ],
                "deploy": "deploy",
                "clean": "clean",
                "destroy": "destroy"
            }
        });
        let diagnostics = validator.validate_class(&broken, ArtifactClass::Provider);
        assert!(
            !diagnostics.is_empty(),
            "a broken inline task body must be rejected via the resolved cross-file task $ref"
        );
    }

    #[test]
    fn a_malformed_artifact_yields_a_diagnostic_naming_the_failing_path_not_a_panic() {
        // Negative space: a nested violation (a task with an unknown `type`) must be reported as a
        // Diagnostic whose path points at the failing location — never a panic.
        let validator = validator();
        let malformed = json!({ "tasks": [ { "name": "x", "type": "bogus-type-here" } ] });

        let diagnostics = validator.validate_class(&malformed, ArtifactClass::Flow);
        assert!(
            !diagnostics.is_empty(),
            "a malformed flow must produce at least one diagnostic"
        );
        assert!(
            diagnostics.iter().all(|d| d.path.is_some()),
            "every schema diagnostic names the failing JSON path: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.path.as_deref().is_some_and(|p| p.contains("/tasks"))),
            "the failing path names the offending `tasks` location: {diagnostics:?}"
        );
    }

    #[test]
    fn a_missing_required_field_reports_the_root_path() {
        // An empty object is missing `tasks`; the violation is at the document root, reported as the
        // root pointer rather than an empty string.
        let validator = validator();
        let diagnostics = validator.validate_class(&json!({}), ArtifactClass::Flow);
        assert!(
            !diagnostics.is_empty(),
            "a flow with no `tasks` is rejected"
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.path.as_deref() == Some(ROOT_POINTER)),
            "a root-level violation is reported at the root pointer: {diagnostics:?}"
        );
    }

    #[test]
    fn validate_maps_each_port_kind_to_its_class() {
        // The port `validate` covers the four non-task classes; each must accept its own artifact.
        let validator = validator();
        let flow = json!({ "tasks": [ { "name": "t", "type": "exec", "with": { "command": "echo hi" } } ] });
        let provider = load_and_classify(&examples_dir().join("provider-manifest.yaml")).1;

        assert!(
            validator
                .validate(&flow, ArtifactKind::Flow)
                .expect("validate does not fault")
                .is_empty(),
            "the port validates a Flow via ArtifactKind::Flow"
        );
        assert!(
            validator
                .validate(&provider, ArtifactKind::Provider)
                .expect("validate does not fault")
                .is_empty(),
            "the port validates a provider via ArtifactKind::Provider"
        );
        // The open `environment` schema accepts any object; exercise that mapping with a real
        // environment object, and use the required-field-bearing Provider schema for the reject side.
        assert!(
            validator
                .validate(
                    &json!({ "name": "dev", "provider": "local-docker" }),
                    ArtifactKind::Environment
                )
                .expect("validate does not fault")
                .is_empty(),
            "the port validates an Environment via ArtifactKind::Environment"
        );
        assert!(
            !validator
                .validate(&json!({}), ArtifactKind::Provider)
                .expect("validate does not fault")
                .is_empty(),
            "an empty object is not a valid provider (missing name/type/methods)"
        );
    }

    #[test]
    fn validate_produces_checks_output_against_a_produces_schema() {
        let validator = validator();
        let schema = json!({
            "type": "object",
            "required": ["count"],
            "properties": { "count": { "type": "integer" } }
        });

        assert!(
            validator
                .validate_produces(&json!({ "count": 3 }), &schema)
                .expect("compiles")
                .is_empty(),
            "an output matching the produces schema is accepted"
        );
        let diagnostics = validator
            .validate_produces(&json!({ "count": "three" }), &schema)
            .expect("compiles");
        assert!(
            !diagnostics.is_empty(),
            "an output violating the produces schema is rejected with a diagnostic"
        );
        assert!(
            diagnostics.iter().all(|d| d.path.is_some()),
            "a produces diagnostic names the failing path: {diagnostics:?}"
        );
    }

    #[test]
    fn validate_produces_faults_on_an_uncompilable_schema() {
        // Negative space: a `produces` schema that is not a valid schema is an internal Validation
        // RunError, not a panic and not a silent accept.
        let validator = validator();
        let broken = json!({ "type": 12345 }); // `type` must be a string or array of strings
        let error = validator
            .validate_produces(&json!({}), &broken)
            .expect_err("an uncompilable produces schema faults");
        assert_eq!(
            error.category,
            tmx_core::ErrorCategory::Validation,
            "an uncompilable produces schema is a Validation-category fault"
        );
        assert_eq!(
            error.code, "produces_schema_uncompilable",
            "the fault carries the stable produces-uncompilable code"
        );
    }
}
