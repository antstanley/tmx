//! The JSON front-end of the [`SourceLoader`](crate::loader::FileSourceLoader): parse a plain JSON
//! document straight into the shared [`serde_json::Value`] model.
//!
//! JSON is the model's native encoding, so this loader is a thin, lossless `from_str` — the other
//! three formats (YAML, JSONC, TOML) are defined by producing the *same* `Value` this one does.

use serde_json::Value;

use tmx_core::error::RunError;

/// Parse `text` as JSON into the shared [`Value`] model. A syntax error is a typed
/// [`ErrorCategory::Validation`](tmx_core::error::ErrorCategory::Validation) `RunError`
/// (`code: source_parse_error`), never a panic.
pub(crate) fn parse(text: &str) -> Result<Value, RunError> {
    serde_json::from_str(text)
        .map_err(|e| RunError::validation("source_parse_error", format!("invalid JSON: {e}")))
}
