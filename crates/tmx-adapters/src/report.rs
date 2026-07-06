//! [`StderrProgressSink`] — the minimal human-progress [`EventSink`] adapter.
//!
//! The stdout / stderr split of 07 §stdout / stderr contract: machine data (the final Pipeline state)
//! goes to **stdout**, human progress to **stderr**. This sink renders each domain [`Event`] as a
//! short progress line on stderr, so `tmx run flow.yaml | jq` sees only the JSON object on stdout
//! while the operator still watches per-task progress. The runner masks every event *before* it
//! reaches a sink, so a rendered line can never leak a secret. The full `--format pretty|json|ndjson`
//! reporter surface is task 26; this is the one always-on progress reporter task 17 needs.

use std::io::Write;

use tmx_core::ports::driven::EventSink;
use tmx_core::{Event, RunError};

/// An [`EventSink`] that writes a short human-progress line per event to a writer (stderr in
/// production). Generic over the writer so a test can capture the rendered stream.
#[derive(Debug, Default)]
pub struct StderrProgressSink;

impl StderrProgressSink {
    /// A fresh progress sink writing to the process's stderr.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Render one (already-masked) [`Event`] as a human progress line, or `None` for events that carry no
/// operator-facing progress (a bare `log.truncated`, the fan-out sub-events task 17 does not run).
#[must_use]
pub fn render_event(event: &Event) -> Option<String> {
    match event {
        Event::RunStart { id, flow } => Some(format!("run: start {flow} ({id})")),
        Event::RunFinish { status, ms, .. } => {
            Some(format!("run: {} in {}ms", status.as_str(), ms.0))
        }
        Event::TaskStart { name } => Some(format!("task: start {name}")),
        Event::TaskFinish {
            name, status, ms, ..
        } => Some(format!("task: {} {name} ({}ms)", status.as_str(), ms.0)),
        Event::TaskSkip { name, reason } => Some(format!("task: skip {name} ({reason})")),
        Event::TaskError { name, error } => Some(format!("task: error {name}: {}", error.message)),
        _ => None,
    }
}

#[async_trait::async_trait]
impl EventSink for StderrProgressSink {
    async fn emit(&self, event: &Event) -> Result<(), RunError> {
        if let Some(line) = render_event(event) {
            // Progress is best-effort: a stderr write failure must not fail the run (stdout data is
            // the contract), so a broken pipe on stderr is swallowed rather than raised.
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(stderr, "{line}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmx_core::{Milliseconds, RunId, RunStatus, TaskStatus};

    const VALID_UUID_V7: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

    #[test]
    fn renders_the_lifecycle_events_to_progress_lines() {
        let id = RunId::new(VALID_UUID_V7).expect("valid id");
        assert_eq!(
            render_event(&Event::RunStart {
                id: id.clone(),
                flow: "deploy".to_string(),
            })
            .as_deref(),
            Some("run: start deploy (018f8c7e-9b2a-7def-8123-456789abcdef)"),
            "run.start names the flow and id"
        );
        assert_eq!(
            render_event(&Event::TaskFinish {
                name: "build".to_string(),
                status: TaskStatus::Ok,
                ms: Milliseconds(7),
                output: None,
            })
            .as_deref(),
            Some("task: ok build (7ms)"),
            "task.finish names the outcome and duration"
        );
        assert_eq!(
            render_event(&Event::RunFinish {
                id,
                status: RunStatus::Failed,
                ms: Milliseconds(42),
            })
            .as_deref(),
            Some("run: failed in 42ms"),
            "run.finish names the terminal status"
        );
    }

    #[test]
    fn events_without_operator_progress_render_to_nothing() {
        // Negative space: a bare log.truncated envelope carries no per-task progress, so it renders to
        // no line rather than an empty or misleading one.
        assert!(
            render_event(&Event::LogTruncated).is_none(),
            "log.truncated produces no progress line"
        );
    }
}
