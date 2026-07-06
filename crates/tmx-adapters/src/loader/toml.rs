//! The TOML front-end: deserialise a TOML document straight into the shared [`serde_json::Value`]
//! model via the `toml` crate, so a `.toml` document lands in the same `Value` as its `.json` twin.
//!
//! TOML types map onto JSON the obvious way — integers to numbers, tables to objects, arrays to
//! arrays — which is what makes cross-format parity hold. (TOML datetimes have no JSON counterpart;
//! the corpus uses none, and one would surface as a typed parse error rather than a silent coercion.)

use serde_json::Value;

use tmx_core::error::RunError;

/// Parse `text` as TOML into the shared [`Value`] model. A syntax error is a typed
/// [`ErrorCategory::Validation`](tmx_core::error::ErrorCategory::Validation) `RunError`
/// (`code: source_parse_error`), never a panic.
pub(crate) fn parse(text: &str) -> Result<Value, RunError> {
    toml::from_str(text)
        .map_err(|e| RunError::validation("source_parse_error", format!("invalid TOML: {e}")))
}
