//! The typed error model the core returns — [`RunError`] and its [`ErrorCategory`].
//!
//! The core knows nothing about process exit codes; it returns a typed [`RunError`] carrying a
//! category, a stable machine `code`, a human `message`, and optional `task`/`path` context
//! ([`.specs/08-errors-and-observability.md` §Error model](../../../.specs/08-errors-and-observability.md)).
//! The CLI adapter alone maps a category to an exit code. `anyhow` is deliberately absent — it would
//! erase the category (08 §Error model); every error is data with a category, not an opaque string.
//!
//! [`ErrorCategory`] and [`RunError`] serialise to the `ErrorCategory` / `RunError` `$def`s in
//! [`.specs/canonical-types.schema.json`](../../../.specs/canonical-types.schema.json). The category
//! is a **closed** enum — there is no catch-all variant, so an out-of-vocabulary category is
//! unrepresentable, and every `match` on it is exhaustive by construction.

use serde::{Deserialize, Serialize};

/// The typed error category the core returns — the `ErrorCategory` `$def`.
///
/// A closed vocabulary of six categories (08 §Error model). The CLI maps each to an exit code
/// (`run_failure`→1, `validation`→3, `resolution`→4, `environment`→5, `timeout`→124,
/// `interrupt`→130); the core itself never sees exit codes. Serialises `snake_case`, matching the
/// schema enum exactly. No `Other`/`Unknown` variant exists: an out-of-vocabulary category cannot be
/// constructed, which is the negative space the type enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// A task aborted, an `assert` failed, an `eval` threshold missed, or the state cap was
    /// exceeded. CLI exit code 1.
    RunFailure,
    /// A schema or `lint` failure, including a preflight task-validation failure. CLI exit code 3.
    Validation,
    /// A reference / flow / provider was not found, a `${{ }}` or input was bad, or recursion was
    /// too deep. CLI exit code 4.
    Resolution,
    /// A provider method failed, or a preflight capability check failed. CLI exit code 5.
    Environment,
    /// `--timeout` was exceeded. CLI exit code 124.
    Timeout,
    /// The run was interrupted (SIGINT). CLI exit code 130.
    Interrupt,
}

impl ErrorCategory {
    /// Every category, in declaration order — used by the exhaustiveness test and by callers that
    /// enumerate the vocabulary. Kept in sync with the enum by the `match` in
    /// [`ErrorCategory::as_str`], which has no wildcard arm and so fails to compile if a variant is
    /// added without updating it.
    pub const ALL: [ErrorCategory; 6] = [
        ErrorCategory::RunFailure,
        ErrorCategory::Validation,
        ErrorCategory::Resolution,
        ErrorCategory::Environment,
        ErrorCategory::Timeout,
        ErrorCategory::Interrupt,
    ];

    /// The stable `snake_case` wire token for this category — the exact string it serialises to.
    ///
    /// The `match` is exhaustive with no wildcard: adding a variant without a token is a compile
    /// error, so the wire vocabulary can never silently drift from the enum.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCategory::RunFailure => "run_failure",
            ErrorCategory::Validation => "validation",
            ErrorCategory::Resolution => "resolution",
            ErrorCategory::Environment => "environment",
            ErrorCategory::Timeout => "timeout",
            ErrorCategory::Interrupt => "interrupt",
        }
    }
}

/// A typed failure returned by the core — the `RunError` `$def`.
///
/// Carries its [`category`](RunError::category), a stable machine [`code`](RunError::code) (always a
/// `&'static str` literal — the codes are a closed, compile-time set, never dynamic text), a human
/// [`message`](RunError::message), and optional [`task`](RunError::task) / [`path`](RunError::path)
/// context. Derives [`std::error::Error`] via `thiserror` and `Serialize` (it is emitted, e.g. on a
/// `task.error` event and in a `TaskResult`). It is not `Deserialize`: `code` is `&'static str`, so
/// a value read back from disk is a distinct concern owned by the `RunStore` (task 27), not this
/// emit-only type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{category:?} [{code}]: {message}")]
pub struct RunError {
    /// The category this failure belongs to; drives the CLI exit-code mapping.
    pub category: ErrorCategory,
    /// A stable machine code, e.g. `state_cap_exceeded`, `unknown_namespace`, `flow_depth_exceeded`.
    pub code: &'static str,
    /// A human-readable message describing the failure.
    pub message: String,
    /// The name of the task that produced the error, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The JSON pointer or reference path the error relates to, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl RunError {
    /// Construct a `RunError` in `category` with the stable `code` and a human `message`, no
    /// `task`/`path` context. Use [`with_task`](RunError::with_task) / [`with_path`](RunError::with_path)
    /// to attach context.
    #[must_use]
    pub fn new(category: ErrorCategory, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            category,
            code,
            message: message.into(),
            task: None,
            path: None,
        }
    }

    /// A [`ErrorCategory::Validation`] error — a schema or `lint` failure.
    #[must_use]
    pub fn validation(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Validation, code, message)
    }

    /// A [`ErrorCategory::Resolution`] error — a bad reference, input, or over-deep recursion.
    #[must_use]
    pub fn resolution(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::Resolution, code, message)
    }

    /// A [`ErrorCategory::RunFailure`] error — a task aborted, an assert failed, a cap was exceeded.
    #[must_use]
    pub fn run_failure(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(ErrorCategory::RunFailure, code, message)
    }

    /// Attach the name of the task that produced this error.
    #[must_use]
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    /// Attach the JSON pointer / reference path this error relates to.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_wire_tokens_are_snake_case_and_match_serialisation() {
        // The `as_str` token and the serde wire form must agree for every category — one drives the
        // exit-code mapping, the other the schema. A divergence would silently mis-key a payload.
        for category in ErrorCategory::ALL {
            let serialised = serde_json::to_value(category).expect("category serialises");
            assert_eq!(
                serialised,
                serde_json::Value::String(category.as_str().to_string()),
                "serde wire form must equal as_str token for {category:?}"
            );
        }
        assert_eq!(
            ErrorCategory::RunFailure.as_str(),
            "run_failure",
            "run_failure is the snake_case token, not 'runFailure' or 'run-failure'"
        );
    }

    #[test]
    fn all_covers_every_variant_exactly_once() {
        // ALL must list each variant once: the length is the vocabulary size, and every token is
        // distinct. A duplicated or missing entry would break the exit-code table and the schema.
        assert_eq!(
            ErrorCategory::ALL.len(),
            6,
            "six categories, no more, no fewer"
        );
        let mut tokens: Vec<&str> = ErrorCategory::ALL.iter().map(|c| c.as_str()).collect();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), 6, "every category token is distinct");
    }

    #[test]
    fn run_error_carries_all_five_fields_and_omits_absent_context() {
        let err = RunError::run_failure("state_cap_exceeded", "state exceeded 512 MiB")
            .with_task("upload")
            .with_path("/tasks/upload");
        assert_eq!(
            err.category,
            ErrorCategory::RunFailure,
            "category is preserved"
        );
        assert_eq!(err.code, "state_cap_exceeded", "code is the stable literal");
        assert_eq!(err.task.as_deref(), Some("upload"), "task context attaches");

        let json = serde_json::to_value(&err).expect("RunError serialises");
        assert_eq!(
            json["category"], "run_failure",
            "category serialises snake_case"
        );
        assert_eq!(json["path"], "/tasks/upload", "path context serialises");

        // Negative space: a bare error omits the optional context keys entirely (skip_serializing_if),
        // rather than emitting nulls the schema's additionalProperties:false forbids.
        let bare = RunError::validation("schema_invalid", "bad flow");
        let bare_json = serde_json::to_value(&bare).expect("bare RunError serialises");
        assert!(
            bare_json.get("task").is_none(),
            "absent task is omitted, not null"
        );
        assert!(
            bare_json.get("path").is_none(),
            "absent path is omitted, not null"
        );
    }
}
