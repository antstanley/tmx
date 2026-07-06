//! [`NdjsonSink`] — the machine-readable event reporter, one JSON [`Event`] per line to **stdout**.
//!
//! Selected by `--format ndjson` (07 §stdout / stderr contract): CI, streaming, and programmatic / LLM
//! consumers read the canonical event stream as newline-delimited JSON on stdout while the human
//! progress stays on stderr. Every event is the run Masker's [`Masked`] payload, so no line can carry
//! a secret; the sink asserts that routing before it writes.

use std::io::Write;

use tmx_core::mask::Masked;
use tmx_core::ports::driven::EventSink;
use tmx_core::{Event, RunError};

use super::assert_routed;

/// An [`EventSink`] that serialises each event as one JSON line to the process's stdout.
#[derive(Debug, Default)]
pub struct NdjsonSink;

impl NdjsonSink {
    /// A fresh ndjson sink writing to stdout.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Assert the payload routed through the Masker, then render it to its one-line JSON form. The
    /// pure seam the [`EventSink`] impl and the unit tests share.
    ///
    /// # Errors
    ///
    /// Returns an `event_serialise_failed` [`RunError`] if the event does not serialise to JSON (an
    /// `Event` always does by construction, so this is defensive negative space, never a live path).
    pub fn render_masked(&self, event: &Masked<Event>) -> Result<String, RunError> {
        assert_routed(event, "NdjsonSink");
        serde_json::to_string(event.get()).map_err(|e| {
            RunError::run_failure(
                "event_serialise_failed",
                format!("event did not serialise to ndjson: {e}"),
            )
        })
    }
}

#[async_trait::async_trait]
impl EventSink for NdjsonSink {
    async fn emit(&self, event: &Masked<Event>) -> Result<(), RunError> {
        let line = self.render_masked(event)?;
        // stdout is the data contract: unlike stderr progress, a failed stdout write is surfaced as a
        // typed error rather than swallowed, so a truncated event stream cannot pass silently.
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{line}").map_err(|e| {
            RunError::run_failure(
                "event_write_failed",
                format!("could not write an ndjson event to stdout: {e}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tmx_core::mask::Masker;
    use tmx_core::{Milliseconds, RunId, TaskStatus};

    const VALID_UUID_V7: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

    #[test]
    fn renders_one_json_object_per_event_with_the_event_tag() {
        let masker = Masker::new();
        let sink = NdjsonSink::new();
        let line = sink
            .render_masked(&masker.redact_event(&Event::RunStart {
                id: RunId::new(VALID_UUID_V7).expect("valid id"),
                flow: "deploy".to_string(),
            }))
            .expect("run.start serialises");
        // One line, no embedded newline, valid JSON internally tagged on `event`.
        assert!(!line.contains('\n'), "an ndjson event is a single line");
        let parsed: Value = serde_json::from_str(&line).expect("the line is JSON");
        assert_eq!(parsed["event"], "run.start", "the event tag is present");
        assert_eq!(parsed["flow"], "deploy", "the payload rides beside the tag");
    }

    #[test]
    fn a_secret_in_a_task_output_is_redacted_in_the_ndjson_line() {
        // Masking at the boundary: a secret echoed into a task output never appears in the JSON line.
        let secret = "supersecretndjson";
        let mut masker = Masker::new();
        masker.register(secret);
        let sink = NdjsonSink::new();
        let line = sink
            .render_masked(&masker.redact_event(&Event::TaskFinish {
                name: "leak".to_string(),
                status: TaskStatus::Ok,
                ms: Milliseconds(2),
                output: Some(serde_json::json!({ "message": format!("token {secret}") })),
            }))
            .expect("task.finish serialises");
        assert!(!line.contains(secret), "the secret is redacted: {line}");
        assert!(
            line.contains("[REDACTED]"),
            "a placeholder replaces it: {line}"
        );
    }

    #[test]
    #[should_panic(expected = "did not route through the Masker")]
    fn an_unrouted_payload_trips_the_boundary_assertion() {
        // Negative space: a payload that skipped the Masker trips the per-sink routing assertion.
        let sink = NdjsonSink::new();
        let _ = sink.render_masked(&Masked::unrouted_for_test(Event::TaskStart {
            name: "leak".to_string(),
        }));
    }
}
