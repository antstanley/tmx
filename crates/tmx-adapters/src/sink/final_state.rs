//! [`FinalStateSink`] — the machine-data reporter, the merged final Pipeline state to **stdout**.
//!
//! Selected by `--format json` (the pipe default, 07 §stdout / stderr contract): stdout carries the
//! final Pipeline state as one JSON object, so `tmx run flow.yaml | jq '.build.sha'` works with no
//! flag. Unlike the streaming [`PrettySink`](super::PrettySink) / [`NdjsonSink`](super::NdjsonSink),
//! this is a **terminal** reporter — it renders once, at run end, the whole masked state object,
//! rather than one line per event. The state has already been redacted by the run's
//! [`Masker`](tmx_core::mask::Masker); the sink accepts only a [`Masked<Value>`] and asserts its
//! routing, so no raw state object can reach stdout.

use std::io::Write;

use serde_json::Value;
use tmx_core::RunError;
use tmx_core::mask::Masked;

use super::assert_routed;

/// The reporter that renders the masked final Pipeline state as one JSON object to stdout.
#[derive(Debug, Default)]
pub struct FinalStateSink;

impl FinalStateSink {
    /// A fresh final-state sink writing to stdout.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Assert the state routed through the Masker, then render it as one pretty JSON object. The pure
    /// seam the stdout writer and the unit tests share.
    ///
    /// A state that fails to serialise (unexpected — the state is a JSON object by construction)
    /// renders as `{}` so stdout always carries valid JSON rather than a partial fragment.
    #[must_use]
    pub fn render_masked(&self, state: &Masked<Value>) -> String {
        assert_routed(state, "FinalStateSink");
        serde_json::to_string_pretty(state.get()).unwrap_or_else(|_| "{}".to_string())
    }

    /// Render the masked final state and write it as one line-terminated JSON object to stdout.
    ///
    /// # Errors
    ///
    /// Returns a `state_write_failed` [`RunError`] if the stdout write fails (a broken pipe): stdout
    /// is the data contract, so a failed write is surfaced, never swallowed.
    pub fn emit(&self, state: &Masked<Value>) -> Result<(), RunError> {
        let rendered = self.render_masked(state);
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{rendered}").map_err(|e| {
            RunError::run_failure(
                "state_write_failed",
                format!("could not write the final state to stdout: {e}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tmx_core::mask::Masker;

    #[test]
    fn renders_the_final_state_as_one_pretty_json_object() {
        let masker = Masker::new();
        let sink = FinalStateSink::new();
        let state = json!({ "build": { "message": "built-ok" } });
        let rendered = sink.render_masked(&masker.redact_state(&state));
        let parsed: Value = serde_json::from_str(&rendered).expect("the render is JSON");
        assert_eq!(
            parsed["build"]["message"], "built-ok",
            "the state object is rendered whole"
        );
    }

    #[test]
    fn a_secret_in_the_final_state_is_redacted() {
        // Masking at the boundary: a secret merged into the final state never reaches stdout.
        let secret = "supersecretstate";
        let mut masker = Masker::new();
        masker.register(secret);
        let sink = FinalStateSink::new();
        let state = json!({ "leak": { "message": format!("token {secret}") } });
        let rendered = sink.render_masked(&masker.redact_state(&state));
        assert!(
            !rendered.contains(secret),
            "the secret is redacted: {rendered}"
        );
        assert!(
            rendered.contains("[REDACTED]"),
            "a placeholder replaces it: {rendered}"
        );
    }

    #[test]
    #[should_panic(expected = "did not route through the Masker")]
    fn an_unrouted_state_trips_the_boundary_assertion() {
        // Negative space: a final-state object that skipped the Masker trips the routing assertion.
        let sink = FinalStateSink::new();
        let _ = sink.render_masked(&Masked::unrouted_for_test(json!({ "leak": "raw" })));
    }
}
