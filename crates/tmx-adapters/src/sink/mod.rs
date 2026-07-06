//! The reporter [`EventSink`] adapters — the three renderers of the canonical event stream, plus the
//! `--format` selection they answer to (08 §Events & reporters, 07 §stdout / stderr contract).
//!
//! The WebCLI stream separation is the organising rule: **machine data goes to stdout, human progress
//! to stderr**. Two channels run concurrently during a run:
//!
//! - **stderr — always-on progress.** [`PrettySink`] renders each event as a short human line to
//!   stderr, independent of `--format`. An operator watching a run reads this while a `| jq` pipeline
//!   consumes stdout untouched.
//! - **stdout — the `--format`-selected reporter.** [`Format`] picks what (if anything) stdout
//!   carries: [`Format::Json`] emits the final Pipeline state as one object at the end
//!   ([`FinalStateSink`]); [`Format::Ndjson`] emits one [`Event`] per line as the run streams
//!   ([`NdjsonSink`]); [`Format::Pretty`] emits nothing to stdout — the human reads the stderr
//!   progress. `pretty` is the TTY default, `json` the pipe default.
//!
//! ## Masking at the boundary
//!
//! Every payload a sink emits is a [`Masked`] token the run's [`Masker`] sealed (08 §Masking at the
//! boundary): the [`EventSink`] port accepts only `Masked<Event>`, and [`FinalStateSink`] accepts
//! only `Masked<Value>`. That typestate makes it *structurally* impossible for a sink to emit
//! un-scrubbed data; each sink additionally calls [`assert_routed`] — the paired runtime check that
//! the payload's origin is non-zero — so a payload that bypassed the Masker trips an assertion rather
//! than leaking. A new sink that forgets this cannot compile against a raw payload and fails the
//! assertion in tests if it forges one.
//!
//! [`Masked`]: tmx_core::mask::Masked
//! [`Masker`]: tmx_core::mask::Masker
//! [`Value`]: serde_json::Value

mod final_state;
mod ndjson;
mod pretty;

pub use final_state::FinalStateSink;
pub use ndjson::NdjsonSink;
pub use pretty::{PrettySink, render_event};

use tmx_core::mask::Masked;
use tmx_core::ports::driven::EventSink;
use tmx_core::{Event, RunError};

/// The stdout reporter `--format` selects (07 §stdout / stderr contract). `pretty` is the TTY
/// default, `json` the pipe default; stderr progress is independent of this choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// Human run summary; stdout carries nothing (the human reads the stderr progress). TTY default.
    Pretty,
    /// The final Pipeline state as one masked JSON object on stdout. Pipe default; `tmx run | jq`.
    #[default]
    Json,
    /// One masked [`Event`](tmx_core::Event) per line on stdout — for CI / programmatic / LLM
    /// consumers.
    Ndjson,
}

impl Format {
    /// Every format, in declaration order — exercises the exhaustiveness of [`Format::as_str`].
    pub const ALL: [Format; 3] = [Format::Pretty, Format::Json, Format::Ndjson];

    /// The stable lowercase token for this format (the `--format` value and `TMX_FORMAT` value).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Format::Pretty => "pretty",
            Format::Json => "json",
            Format::Ndjson => "ndjson",
        }
    }

    /// Parse a `--format` / `TMX_FORMAT` token, case-insensitively; `None` for an unknown token.
    #[must_use]
    pub fn parse(token: &str) -> Option<Format> {
        match token.trim().to_ascii_lowercase().as_str() {
            "pretty" => Some(Format::Pretty),
            "json" => Some(Format::Json),
            "ndjson" => Some(Format::Ndjson),
            _ => None,
        }
    }

    /// The default format for the stdout target: `pretty` when stdout is an interactive terminal,
    /// `json` when it is a pipe/file (07 §Configuration — "a TTY check selects `pretty` vs `json`").
    #[must_use]
    pub const fn default_for_tty(stdout_is_tty: bool) -> Format {
        if stdout_is_tty {
            Format::Pretty
        } else {
            Format::Json
        }
    }
}

/// The paired runtime boundary check every sink performs before emitting a payload: assert it routed
/// through the Masker (a non-zero origin). A payload that bypassed the Masker carries origin `0`
/// (only [`Masked::unrouted_for_test`](tmx_core::mask::Masked::unrouted_for_test) mints one) and
/// trips this — so a sink cannot emit un-scrubbed data even if the typestate were somehow bypassed.
pub(crate) fn assert_routed<T>(payload: &Masked<T>, sink: &'static str) {
    assert!(
        payload.origin() != 0,
        "{sink} received a payload that did not route through the Masker"
    );
}

/// The composite streaming [`EventSink`] the runner emits through — the always-on stderr progress plus
/// the `--format`-selected stdout stream, fanned from one event.
///
/// - [`PrettySink`] → **stderr**, under every format (progress is independent of the stdout reporter).
/// - [`NdjsonSink`] → **stdout**, only under [`Format::Ndjson`] (one event per line as the run
///   streams). Under [`Format::Json`] the final state is rendered once at run end by
///   [`FinalStateSink`] — a terminal reporter, not part of this stream — and under [`Format::Pretty`]
///   stdout carries nothing.
///
/// The `--color` choice is threaded into the stderr [`PrettySink`]; the stdout ndjson stream is never
/// coloured (it is machine data).
pub struct ReporterSink {
    /// The always-on human progress, to stderr.
    progress: PrettySink,
    /// The stdout event stream, present only under `--format ndjson`.
    stream: Option<NdjsonSink>,
}

impl ReporterSink {
    /// Build the streaming reporter for `format`, colouring the stderr progress per `color`.
    #[must_use]
    pub const fn for_format(format: Format, color: bool) -> Self {
        Self {
            progress: PrettySink::with_color(color),
            stream: match format {
                Format::Ndjson => Some(NdjsonSink::new()),
                Format::Pretty | Format::Json => None,
            },
        }
    }
}

#[async_trait::async_trait]
impl EventSink for ReporterSink {
    async fn emit(&self, event: &Masked<Event>) -> Result<(), RunError> {
        // Progress first (stderr, best-effort), then the stdout stream when selected. A stdout write
        // failure is surfaced (the data contract), unlike the swallowed stderr progress.
        self.progress.emit(event).await?;
        if let Some(stream) = &self.stream {
            stream.emit(event).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tokens_round_trip_and_reject_unknown() {
        // Every format's token parses back to itself, and the parse is case-insensitive.
        for format in Format::ALL {
            assert_eq!(
                Format::parse(format.as_str()),
                Some(format),
                "{format:?} round-trips through its token"
            );
        }
        assert_eq!(
            Format::parse("  NDJSON "),
            Some(Format::Ndjson),
            "parsing is case-insensitive and trims whitespace"
        );
        // Negative space: an unknown format token is rejected, not silently defaulted.
        assert_eq!(Format::parse("yaml"), None, "an unknown format is rejected");
        assert_eq!(Format::parse(""), None, "the empty token is rejected");
    }

    #[test]
    fn tty_selects_pretty_and_pipe_selects_json() {
        // The TTY default is pretty; the pipe/file default is json (so `tmx run | jq` works with no
        // flag). The struct default matches the pipe case — the machine-data contract.
        assert_eq!(
            Format::default_for_tty(true),
            Format::Pretty,
            "an interactive terminal defaults to pretty"
        );
        assert_eq!(
            Format::default_for_tty(false),
            Format::Json,
            "a pipe/file defaults to json"
        );
        assert_eq!(
            Format::default(),
            Format::Json,
            "the struct default is json"
        );
    }
}
