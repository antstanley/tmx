//! [`RecordingEventSink`] — the capturing [`EventSink`] fake.
//!
//! Records the domain [`Event`] stream in emission order so a test can assert on exactly what the
//! runner reported. Because the events are captured, two runs over the same fresh bundle yield
//! byte-identical streams — the reproducibility the determinism obligation checks.
//!
//! ## Forward reference — Masker routing (task 09)
//!
//! Task 06's step 3 says this sink "asserts it routed every payload through the Masker." The Masker
//! is task 09 and does not exist yet, so that assertion is a **deferred** forward reference: this
//! sink records the raw stream today, and the Masker-routing check is wired when task 09 lands. No
//! behaviour here depends on the Masker, so the omission does not weaken the recording contract.

use std::sync::Mutex;

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
    async fn emit(&self, event: &Event) -> Result<(), RunError> {
        if let Ok(mut events) = self.events.lock() {
            events.push(event.clone());
        }
        Ok(())
    }
}
