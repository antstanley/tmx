//! [`SystemClock`] — the production wall-clock / duration [`Clock`] adapter.
//!
//! The real-time counterpart of the testkit's `FixedClock`. [`now`](Clock::now) reads the system
//! wall clock and renders it as an RFC 3339 UTC instant; [`now_ms`](Clock::now_ms) reads a monotonic
//! counter (a [`std::time::Instant`] delta) so per-task durations are immune to wall-clock steps. The
//! RFC 3339 rendering is done with a small civil-date routine so the adapter pulls in no calendar
//! crate — it depends only on `std::time`.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tmx_core::Milliseconds;
use tmx_core::ports::driven::Clock;

/// Seconds in one day — the divisor that splits a Unix timestamp into whole days and the
/// intra-day remainder. A units-last calendar constant, not a tunable engine limit, so it is a
/// local `const` rather than a `tmx-schema::limits` entry.
const SECONDS_PER_DAY: u64 = 86_400;
/// Seconds in one hour.
const SECONDS_PER_HOUR: u64 = 3_600;
/// Seconds in one minute.
const SECONDS_PER_MINUTE: u64 = 60;
/// The day-count offset from the Unix epoch (1970-01-01) to the civil-from-days era origin
/// (0000-03-01), used by Howard Hinnant's `civil_from_days` algorithm.
const EPOCH_TO_ERA_OFFSET_DAYS: i64 = 719_468;
/// The number of days in a 400-year Gregorian era (the algorithm's repeating cycle).
const DAYS_PER_ERA: i64 = 146_097;

/// The production [`Clock`]: a monotonic origin captured at construction plus the system wall clock.
///
/// `now_ms` reports the milliseconds elapsed since the origin [`Instant`], so it is monotonic and
/// unaffected by wall-clock adjustments; `now` reports the current UTC instant as an RFC 3339 string.
#[derive(Debug, Clone)]
pub struct SystemClock {
    /// The monotonic origin `now_ms` measures from; only differences are meaningful.
    origin: Instant,
}

impl SystemClock {
    /// A clock whose monotonic counter starts now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClock {
    /// The retention cutoff `days_ago` days before now, as an RFC 3339 UTC [`Timestamp`] — the value
    /// the `RunStore`'s prune compares each run's `startedAt` against (08 §Run store: retention). The
    /// subtraction saturates at the epoch rather than underflowing.
    #[must_use]
    pub fn cutoff_days_ago(&self, days_ago: u64) -> tmx_core::Timestamp {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let cutoff_secs = since_epoch
            .as_secs()
            .saturating_sub(days_ago.saturating_mul(SECONDS_PER_DAY));
        tmx_core::Timestamp::new(format_rfc3339(cutoff_secs, since_epoch.subsec_millis()))
    }
}

impl Clock for SystemClock {
    fn now(&self) -> tmx_core::Timestamp {
        // A pre-epoch system clock (duration_since fails) falls back to the epoch rather than
        // panicking — the timestamp is descriptive, not load-bearing.
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        tmx_core::Timestamp::new(format_rfc3339(
            since_epoch.as_secs(),
            since_epoch.subsec_millis(),
        ))
    }

    fn now_ms(&self) -> Milliseconds {
        // Saturate rather than wrap on the (astronomically unlikely) overflow of a u128→u64 cast.
        Milliseconds(u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

/// Render `secs` Unix seconds plus `millis` as an RFC 3339 UTC instant
/// (`YYYY-MM-DDTHH:MM:SS.mmmZ`), computing the civil date with no calendar dependency.
fn format_rfc3339(secs: u64, millis: u32) -> String {
    let days = (secs / SECONDS_PER_DAY) as i64;
    let rem = secs % SECONDS_PER_DAY;
    let hour = rem / SECONDS_PER_HOUR;
    let minute = (rem % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let second = rem % SECONDS_PER_MINUTE;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Convert a count of days since the Unix epoch into a `(year, month, day)` civil date
/// (Howard Hinnant's `civil_from_days`, valid across the whole proleptic Gregorian range).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + EPOCH_TO_ERA_OFFSET_DAYS;
    let era = (if z >= 0 { z } else { z - (DAYS_PER_ERA - 1) }) / DAYS_PER_ERA;
    let doe = z - era * DAYS_PER_ERA; // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era, [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year (Mar-based), [0, 365]
    let mp = (5 * doy + 2) / 153; // month, Mar-based [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_rfc3339_renders_known_instants() {
        // The epoch and a well-known later instant both render exactly, proving the civil-date math.
        assert_eq!(
            format_rfc3339(0, 0),
            "1970-01-01T00:00:00.000Z",
            "the epoch"
        );
        // 2021-01-01T00:00:00Z is 1_609_459_200 Unix seconds.
        assert_eq!(
            format_rfc3339(1_609_459_200, 0),
            "2021-01-01T00:00:00.000Z",
            "a leap-cycle-spanning date"
        );
        // Milliseconds and intra-day fields are placed correctly.
        assert_eq!(
            format_rfc3339(1_609_459_200 + 3_661, 123),
            "2021-01-01T01:01:01.123Z",
            "hours/minutes/seconds/millis all render"
        );
    }

    #[test]
    fn now_ms_is_monotonic_and_now_is_rfc3339() {
        let clock = SystemClock::new();
        let first = clock.now_ms();
        let second = clock.now_ms();
        assert!(
            second.0 >= first.0,
            "the monotonic counter never goes backwards"
        );
        let instant = clock.now();
        let text = instant.as_str();
        assert!(
            text.ends_with('Z') && text.contains('T'),
            "now() renders an RFC 3339 UTC instant, got {text:?}"
        );
        assert_eq!(text.len(), 24, "the rendered instant has the fixed width");
    }

    #[test]
    fn cutoff_days_ago_is_an_earlier_fixed_width_instant() {
        // The retention cutoff is a well-formed RFC 3339 instant strictly before now, and a larger
        // window yields an earlier cutoff (the comparison the RunStore prunes on is chronological).
        let clock = SystemClock::new();
        let now = clock.now();
        let cutoff = clock.cutoff_days_ago(30);
        assert_eq!(cutoff.as_str().len(), 24, "the cutoff has the fixed width");
        assert!(
            cutoff.as_str() < now.as_str(),
            "a 30-day cutoff precedes now ({} < {})",
            cutoff.as_str(),
            now.as_str()
        );
        let further = clock.cutoff_days_ago(365);
        assert!(
            further.as_str() < cutoff.as_str(),
            "a wider window yields an earlier cutoff"
        );
        // Negative space: a saturating window never underflows past the epoch.
        let huge = clock.cutoff_days_ago(u64::MAX);
        assert!(
            huge.as_str().starts_with("1970-01-01"),
            "an enormous window saturates at the epoch, not underflow: {}",
            huge.as_str()
        );
    }
}
