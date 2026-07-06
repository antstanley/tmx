//! The YAML front-end: deserialise a YAML document straight into the shared [`serde_json::Value`]
//! model via `serde_yaml_ng`, so a `.yaml` document lands in the same `Value` as its `.json` twin.
//!
//! YAML's mapping order is visited in document order, which — with `serde_json`'s `preserve_order`
//! feature on `Value` — means a task *map* form keeps its source key order through this loader.

use serde_json::Value;

use tmx_core::error::RunError;

/// Parse `text` as YAML into the shared [`Value`] model. A syntax error is a typed
/// [`ErrorCategory::Validation`](tmx_core::error::ErrorCategory::Validation) `RunError`
/// (`code: source_parse_error`), never a panic.
pub(crate) fn parse(text: &str) -> Result<Value, RunError> {
    serde_yaml_ng::from_str(text)
        .map_err(|e| RunError::validation("source_parse_error", format!("invalid YAML: {e}")))
}
