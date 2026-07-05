//! [`FixedClock`] — the frozen, step-advanceable [`Clock`] fake.
//!
//! The wall-clock determinism seam. `now()` returns a frozen RFC 3339 instant and `now_ms()` a
//! monotonic counter that only moves when a test explicitly [`advances`](FixedClock::advance_ms) it,
//! so a run measures the exact durations the test dictates. Two fresh clocks built from the same
//! seed emit identical sequences — the reproducibility the fan-out and the reporters rely on.

use std::sync::atomic::{AtomicU64, Ordering};

use tmx_core::Milliseconds;
use tmx_core::ports::driven::Clock;

/// The default frozen wall-clock instant a [`FixedClock::new`] reports — a fixed RFC 3339 UTC time.
///
/// A deterministic test fixture, not an engine limit, so it lives here rather than in
/// `tmx-schema::limits` (which is reserved for bounded engine dimensions).
const DEFAULT_INSTANT_RFC3339: &str = "2026-07-05T00:00:00Z";

/// The default monotonic origin a [`FixedClock::new`] starts `now_ms()` from, in milliseconds.
const DEFAULT_ORIGIN_MS: u64 = 0;

/// A frozen [`Clock`]: `now()` is constant, `now_ms()` advances only when stepped.
///
/// The wall-clock instant is fixed for the clock's lifetime. The monotonic counter starts at a
/// configured origin and moves forward only through [`advance_ms`](FixedClock::advance_ms), so a
/// test scripts elapsed time exactly. Interior mutability (`&self` + [`AtomicU64`]) keeps the port's
/// shared-reference signature while allowing the counter to be stepped.
#[derive(Debug)]
pub struct FixedClock {
    instant: String,
    ms: AtomicU64,
}

impl FixedClock {
    /// A frozen clock at [`DEFAULT_INSTANT_RFC3339`] with the monotonic counter at
    /// [`DEFAULT_ORIGIN_MS`]. Two such clocks read identically until one is stepped.
    #[must_use]
    pub fn new() -> Self {
        Self::at(DEFAULT_INSTANT_RFC3339, DEFAULT_ORIGIN_MS)
    }

    /// A frozen clock at a caller-chosen RFC 3339 `instant` with the counter at `origin_ms`.
    #[must_use]
    pub fn at(instant: impl Into<String>, origin_ms: u64) -> Self {
        Self {
            instant: instant.into(),
            ms: AtomicU64::new(origin_ms),
        }
    }

    /// Step the monotonic counter forward by `delta_ms`, returning the new value. The frozen
    /// wall-clock instant is unaffected — only measured durations move.
    pub fn advance_ms(&self, delta_ms: u64) -> Milliseconds {
        // `fetch_add` returns the *previous* value; the new reading is previous + delta.
        let previous = self.ms.fetch_add(delta_ms, Ordering::SeqCst);
        Milliseconds(previous.saturating_add(delta_ms))
    }
}

impl Default for FixedClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FixedClock {
    fn now(&self) -> tmx_core::Timestamp {
        tmx_core::Timestamp::new(self.instant.clone())
    }

    fn now_ms(&self) -> Milliseconds {
        Milliseconds(self.ms.load(Ordering::SeqCst))
    }
}
