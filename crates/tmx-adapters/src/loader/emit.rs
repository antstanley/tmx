//! Re-emit the shared [`Value`] model back to a wire format — the serialising counterpart of the
//! per-format parsers, backing `tmx fmt`'s loss-free format conversion (07 §`tmx fmt`).
//!
//! The round-trip is loss-free **at the model level**: parsing the emitted text yields the same
//! [`Value`] the source parsed to. JSONC comments are not part of the model, so emitting a JSONC
//! document as canonical JSON drops only comments/formatting, never data — a re-parse of either
//! form lands in the identical `Value`.

use serde_json::Value;

use tmx_core::error::RunError;
use tmx_core::ports::driven::SourceKind;

/// Serialise `value` back to `kind`'s wire format. A serialisation failure (a value the target
/// format cannot represent, e.g. a bare `null` under TOML) is a typed
/// [`ErrorCategory::Validation`](tmx_core::error::ErrorCategory::Validation) `RunError`
/// (`code: source_emit_error`), never a panic.
///
/// # Errors
///
/// Returns `source_emit_error` when the target serialiser rejects the value.
pub fn emit_source(value: &Value, kind: SourceKind) -> Result<String, RunError> {
    match kind {
        // JSONC is a superset of JSON; re-emitting canonical JSON parses back identically under the
        // JSONC front-end, so the two share one emitter.
        SourceKind::Json | SourceKind::Jsonc => emit_json(value),
        SourceKind::Yaml => emit_yaml(value),
        SourceKind::Toml => emit_toml(value),
    }
}

/// Emit canonical, pretty-printed JSON with a trailing newline.
fn emit_json(value: &Value) -> Result<String, RunError> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| {
        RunError::validation("source_emit_error", format!("could not emit JSON: {e}"))
    })?;
    text.push('\n');
    Ok(text)
}

/// Emit a YAML document via `serde_yaml_ng` (the same front-end the parser uses).
fn emit_yaml(value: &Value) -> Result<String, RunError> {
    serde_yaml_ng::to_string(value)
        .map_err(|e| RunError::validation("source_emit_error", format!("could not emit YAML: {e}")))
}

/// Emit a TOML document. TOML requires a table at the top level and forbids a scalar following a
/// table within one table, so a value whose top-level keys interleave scalars after tables is a
/// typed error rather than a silent reordering.
fn emit_toml(value: &Value) -> Result<String, RunError> {
    toml::to_string_pretty(value)
        .map_err(|e| RunError::validation("source_emit_error", format!("could not emit TOML: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::parse_source;
    use serde_json::json;

    /// A model whose top-level keys list scalars before tables/array-of-tables, so every target
    /// format (TOML included) can represent it.
    fn sample() -> Value {
        json!({
            "name": "demo",
            "version": "1",
            "inputs": { "count": { "type": "number" } },
            "tasks": [
                { "name": "build", "type": "exec", "with": { "command": "echo hi" } }
            ]
        })
    }

    #[test]
    fn round_trips_loss_free_across_every_format() {
        // Emit the one model to each format, re-parse, and confirm the reloaded model is identical —
        // TMX's defining property (four formats, one model) proven for the emit direction.
        let model = sample();
        for kind in [
            SourceKind::Json,
            SourceKind::Jsonc,
            SourceKind::Yaml,
            SourceKind::Toml,
        ] {
            let text = emit_source(&model, kind).expect("the model emits");
            let reloaded = parse_source(&text, kind).expect("the emitted text re-parses");
            assert_eq!(
                reloaded, model,
                "a {kind:?} round-trip preserves the model exactly"
            );
        }
    }

    #[test]
    fn jsonc_and_json_emit_the_same_canonical_text() {
        // JSONC has no separate emitter: both land in the same canonical JSON, so a JSONC re-emit is
        // parseable as JSON too (the superset relationship the loader relies on).
        let model = sample();
        let as_json = emit_source(&model, SourceKind::Json).expect("json emits");
        let as_jsonc = emit_source(&model, SourceKind::Jsonc).expect("jsonc emits");
        assert_eq!(as_json, as_jsonc, "JSONC re-emits as canonical JSON");
        assert!(
            as_json.ends_with('\n'),
            "JSON emit carries a trailing newline"
        );
    }

    #[test]
    fn toml_rejects_a_value_it_cannot_represent() {
        // Negative space: a bare top-level array is not a TOML table, so the emit is a typed
        // `source_emit_error`, never a panic or a silent drop.
        let err = emit_source(&json!([1, 2, 3]), SourceKind::Toml)
            .expect_err("a non-table document cannot be TOML");
        assert_eq!(err.code, "source_emit_error", "a typed emit error");
        assert_eq!(
            err.category,
            tmx_core::error::ErrorCategory::Validation,
            "an un-emittable value is a validation-category error"
        );
    }
}
