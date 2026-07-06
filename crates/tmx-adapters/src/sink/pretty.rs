//! [`PrettySink`] — the always-on human-progress [`EventSink`], rendering to **stderr**.
//!
//! One short line per event, so an operator watching a run sees per-task progress while a `| jq`
//! pipeline consumes stdout untouched (07 §stdout / stderr contract). It runs under every `--format`:
//! stderr progress is independent of the stdout reporter. Progress is best-effort — a broken pipe on
//! stderr must never fail a run whose data contract is stdout — so a stderr write error is swallowed.
//!
//! ## Colour
//!
//! The status token is ANSI-coloured only when colour is enabled (`--color`, or a colour-capable TTY
//! with neither `--no-color` nor `NO_COLOR` set, resolved by the CLI). `NO_COLOR` / `--no-color`
//! disable it, so the plain-text rendering is a strict prefix-free subset — a pipe never sees escape
//! codes. [`render_event`] is the colourless base rendering the tests pin.

use std::io::Write;

use tmx_core::mask::Masked;
use tmx_core::ports::driven::EventSink;
use tmx_core::{Event, RunError, RunStatus, TaskStatus};

use super::assert_routed;

/// The ANSI SGR reset sequence — closes any colour opened for a status token.
const ANSI_RESET: &str = "\x1b[0m";
/// The ANSI SGR code for a successful (`ok`) status.
const ANSI_GREEN: &str = "\x1b[32m";
/// The ANSI SGR code for a failed/error status.
const ANSI_RED: &str = "\x1b[31m";
/// The ANSI SGR code for a skipped/neutral status.
const ANSI_YELLOW: &str = "\x1b[33m";

/// An [`EventSink`] that writes a short human-progress line per event to the process's stderr.
///
/// `color` decides whether the status token is ANSI-coloured; the CLI resolves it from
/// `--color`/`--no-color`/`NO_COLOR` and the stderr TTY check. [`PrettySink::new`] is colourless, so
/// the default rendering (and every unit test that builds one) is plain text.
#[derive(Debug, Default)]
pub struct PrettySink {
    color: bool,
}

impl PrettySink {
    /// A fresh, colourless progress sink writing to stderr.
    #[must_use]
    pub const fn new() -> Self {
        Self { color: false }
    }

    /// A progress sink whose status tokens are ANSI-coloured when `color` is set.
    #[must_use]
    pub const fn with_color(color: bool) -> Self {
        Self { color }
    }

    /// Assert the payload routed through the Masker, then render it to its progress line (or `None`
    /// for an event that carries no operator-facing progress). The pure seam the [`EventSink`] impl
    /// and the unit tests share — the tests exercise the masking assertion without touching stderr.
    #[must_use]
    pub fn render_masked(&self, event: &Masked<Event>) -> Option<String> {
        assert_routed(event, "PrettySink");
        render_line(event.get(), self.color)
    }
}

/// Render one (already-masked) [`Event`] as a colourless human progress line — the base rendering the
/// tests pin. `None` only for events with no operator-facing progress (none in the current set).
#[must_use]
pub fn render_event(event: &Event) -> Option<String> {
    render_line(event, false)
}

/// Render one (already-masked) [`Event`] as a human progress line, optionally colouring the status
/// token. Exhaustive over every [`Event`] variant — a new variant must decide its progress rendering
/// here rather than silently rendering to nothing.
fn render_line(event: &Event, color: bool) -> Option<String> {
    match event {
        Event::RunStart { id, flow } => Some(format!("run: start {flow} ({id})")),
        Event::RunFinish { status, ms, .. } => Some(format!(
            "run: {} in {}ms",
            paint(status.as_str(), run_color(*status), color),
            ms.0
        )),
        Event::TaskStart { name } => Some(format!("task: start {name}")),
        Event::TaskFinish {
            name, status, ms, ..
        } => Some(format!(
            "task: {} {name} ({}ms)",
            paint(status.as_str(), task_color(*status), color),
            ms.0
        )),
        Event::TaskSkip { name, reason } => Some(format!(
            "task: {} {name} ({reason})",
            paint("skip", ANSI_YELLOW, color)
        )),
        Event::TaskError { name, error } => Some(format!(
            "task: {} {name}: {}",
            paint("error", ANSI_RED, color),
            error.message
        )),
        Event::MapItemFinish { name, index, ms } => {
            Some(format!("map: {name}[{index}] ({}ms)", ms.0))
        }
        Event::EvalCaseFinish { name, index } => Some(format!("eval: {name} case {index}")),
        Event::HookStart { name } => Some(format!("hook: start {name}")),
        Event::HookFinish { name, status, ms } => Some(format!(
            "hook: {} {name} ({}ms)",
            paint(status.as_str(), task_color(*status), color),
            ms.0
        )),
        // The per-run event log hit its persistence cap; streaming continues, so surface it as a note.
        Event::LogTruncated => Some("log: truncated (persistence cap reached)".to_string()),
    }
}

/// Wrap `token` in an ANSI colour when `color` is set, else return it unchanged — so a pipe (colour
/// off) never sees an escape sequence.
fn paint(token: &str, code: &str, color: bool) -> String {
    if color {
        format!("{code}{token}{ANSI_RESET}")
    } else {
        token.to_string()
    }
}

/// The status colour for a terminal run status.
const fn run_color(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Ok => ANSI_GREEN,
        RunStatus::Failed | RunStatus::TimedOut => ANSI_RED,
        RunStatus::Cancelled => ANSI_YELLOW,
        RunStatus::Pending | RunStatus::Running => ANSI_YELLOW,
    }
}

/// The status colour for a task/hook status.
const fn task_color(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Ok => ANSI_GREEN,
        TaskStatus::Error => ANSI_RED,
        TaskStatus::Skipped => ANSI_YELLOW,
    }
}

#[async_trait::async_trait]
impl EventSink for PrettySink {
    async fn emit(&self, event: &Masked<Event>) -> Result<(), RunError> {
        if let Some(line) = self.render_masked(event) {
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
    use tmx_core::mask::Masker;
    use tmx_core::{Milliseconds, RunId};

    const VALID_UUID_V7: &str = "018f8c7e-9b2a-7def-8123-456789abcdef";

    fn masker() -> Masker {
        Masker::new()
    }

    #[test]
    fn renders_the_lifecycle_events_to_progress_lines() {
        let masker = masker();
        let id = RunId::new(VALID_UUID_V7).expect("valid id");
        let sink = PrettySink::new();
        assert_eq!(
            sink.render_masked(&masker.redact_event(&Event::RunStart {
                id: id.clone(),
                flow: "deploy".to_string(),
            }))
            .as_deref(),
            Some("run: start deploy (018f8c7e-9b2a-7def-8123-456789abcdef)"),
            "run.start names the flow and id"
        );
        assert_eq!(
            sink.render_masked(&masker.redact_event(&Event::TaskFinish {
                name: "build".to_string(),
                status: TaskStatus::Ok,
                ms: Milliseconds(7),
                output: None,
            }))
            .as_deref(),
            Some("task: ok build (7ms)"),
            "task.finish names the outcome and duration"
        );
        assert_eq!(
            sink.render_masked(&masker.redact_event(&Event::RunFinish {
                id,
                status: RunStatus::Failed,
                ms: Milliseconds(42),
            }))
            .as_deref(),
            Some("run: failed in 42ms"),
            "run.finish names the terminal status"
        );
    }

    #[test]
    fn every_event_variant_renders_a_line() {
        // The full event set has an operator-facing rendering (including the fan-out and hook events),
        // so a widened stream never silently drops a line.
        let masker = masker();
        let sink = PrettySink::new();
        let events = [
            Event::TaskSkip {
                name: "gamma".to_string(),
                reason: "if=false".to_string(),
            },
            Event::MapItemFinish {
                name: "fan".to_string(),
                index: 2,
                ms: Milliseconds(5),
            },
            Event::EvalCaseFinish {
                name: "score".to_string(),
                index: 1,
            },
            Event::HookStart {
                name: "create".to_string(),
            },
            Event::HookFinish {
                name: "create".to_string(),
                status: TaskStatus::Ok,
                ms: Milliseconds(1),
            },
            Event::LogTruncated,
        ];
        for event in events {
            assert!(
                sink.render_masked(&masker.redact_event(&event)).is_some(),
                "{event:?} renders a progress line"
            );
        }
    }

    #[test]
    fn color_wraps_the_status_token_only_when_enabled() {
        // NO_COLOR / --no-color honoured: the colourless sink emits no escape sequence, and a pipe
        // therefore never sees one; --color wraps the status token in ANSI. Both carry the same text.
        let masker = masker();
        let event = masker.redact_event(&Event::TaskFinish {
            name: "build".to_string(),
            status: TaskStatus::Ok,
            ms: Milliseconds(3),
            output: None,
        });
        let plain = PrettySink::with_color(false)
            .render_masked(&event)
            .expect("renders");
        let colored = PrettySink::with_color(true)
            .render_masked(&event)
            .expect("renders");
        assert!(
            !plain.contains('\x1b'),
            "colour off emits no escape: {plain:?}"
        );
        assert!(
            colored.contains(ANSI_GREEN) && colored.contains(ANSI_RESET),
            "colour on wraps the ok status in green: {colored:?}"
        );
        assert!(
            colored.contains("build"),
            "the task name is present either way"
        );
    }

    #[test]
    fn a_secret_in_a_task_error_is_redacted_in_the_pretty_line() {
        // Masking at the boundary: a secret echoed into a task.error message never appears in the
        // rendered progress line — the sink only ever sees the masked payload.
        let secret = "supersecretpretty";
        let mut masker = masker();
        masker.register(secret);
        let sink = PrettySink::new();
        let line = sink
            .render_masked(&masker.redact_event(&Event::TaskError {
                name: "leak".to_string(),
                error: RunError::run_failure("boom", format!("crashed with {secret}")),
            }))
            .expect("task.error renders a line");
        assert!(!line.contains(secret), "the secret is redacted: {line}");
        assert!(
            line.contains("[REDACTED]"),
            "a placeholder replaces it: {line}"
        );
    }

    #[test]
    #[should_panic(expected = "did not route through the Masker")]
    fn an_unrouted_payload_trips_the_boundary_assertion() {
        // Negative space: a payload that bypassed the Masker (origin 0) trips the per-sink routing
        // assertion rather than being rendered — a sink cannot emit un-scrubbed data.
        let sink = PrettySink::new();
        let _ = sink.render_masked(&Masked::unrouted_for_test(Event::TaskStart {
            name: "leak".to_string(),
        }));
    }
}
