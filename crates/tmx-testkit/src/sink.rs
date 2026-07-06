//! [`RecordingEventSink`] — the capturing [`EventSink`] fake.
//!
//! Records the domain [`Event`] stream in emission order so a test can assert on exactly what the
//! runner reported. Because the events are captured, two runs over the same fresh bundle yield
//! byte-identical streams — the reproducibility the determinism obligation checks.
//!
//! ## Masker routing
//!
//! Task 06's step 3 says this sink "asserts it routed every payload through the Masker." The
//! [`EventSink`] port now carries a [`Masked<Event>`] (task 26), so this sink asserts the payload's
//! non-zero origin — the paired runtime boundary check every real sink performs — before recording
//! the inner event. A payload that skipped the Masker (origin `0`) trips the assertion, so the
//! recording fake cannot silently capture un-routed data.

use std::sync::Mutex;

use tmx_core::mask::Masked;
use tmx_core::ports::driven::EventSink;
use tmx_core::{Event, RunError};

/// An [`EventSink`] that captures every emitted [`Event`] in order.
#[derive(Debug, Default)]
pub struct RecordingEventSink {
    events: Mutex<Vec<Event>>,
}

impl RecordingEventSink {
    /// An empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The captured events, in emission order.
    #[must_use]
    pub fn events(&self) -> Vec<Event> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    /// The captured stream serialised as ndjson (one JSON event per line), for byte-level diffing
    /// across runs. A serialisation fault is surfaced as a typed [`RunError`] rather than a panic.
    pub fn ndjson(&self) -> Result<String, RunError> {
        let events = self.events();
        let mut out = String::new();
        for event in &events {
            let line = serde_json::to_string(event).map_err(|e| {
                RunError::run_failure(
                    "event_serialise_failed",
                    format!("event did not serialise: {e}"),
                )
            })?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl EventSink for RecordingEventSink {
    async fn emit(&self, event: &Masked<Event>) -> Result<(), RunError> {
        // The output-port half of the masking boundary: a recorded payload must have routed through
        // the Masker (a non-zero origin). A forged un-routed payload trips this, so the fake cannot
        // capture data that bypassed redaction.
        assert!(
            event.origin() != 0,
            "RecordingEventSink received an event that did not route through the Masker"
        );
        if let Ok(mut events) = self.events.lock() {
            events.push(event.get().clone());
        }
        Ok(())
    }
}
