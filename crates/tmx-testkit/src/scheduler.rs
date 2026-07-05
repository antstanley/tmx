//! [`SerialScheduler`] — the strictly-serial, index-ordered [`Scheduler`] fake.
//!
//! The determinism seam for all fan-out (`map`/`eval`). The production adapter (task 18) runs up to
//! `concurrency` units in flight behind a semaphore and collates by index; this fake runs them one
//! at a time, in index order, so a run's output is byte-for-byte reproducible regardless of how the
//! underlying futures would have interleaved. It still honours the port contract: exactly `count`
//! results, in **index** order, with `concurrency >= 1` and in-flight `<= concurrency` asserted.

use std::future::Future;

use tmx_core::RunError;
use tmx_core::ports::driven::Scheduler;
use tmx_schema::limits::CONCURRENCY_MAX;

/// A strictly-serial [`Scheduler`]: runs `make(0), make(1), … make(count-1)` one at a time and
/// collects the results in index order.
///
/// Deterministic by construction — there is never more than one unit in flight, so completion order
/// equals submission order and a run's result vector is reproducible. The `concurrency` argument is
/// still honoured as a *bound* (asserted `>= 1`, `<= `[`CONCURRENCY_MAX`], and never exceeded by the
/// single in-flight unit), so a caller that mis-sizes the budget is caught here rather than in the
/// production adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct SerialScheduler;

impl SerialScheduler {
    /// Construct a serial scheduler. Stateless: two instances behave identically.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Scheduler for SerialScheduler {
    async fn run_indexed<T, F, Fut>(
        &self,
        count: u32,
        concurrency: u32,
        make: F,
    ) -> Vec<Result<T, RunError>>
    where
        T: Send,
        F: Fn(u32) -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, RunError>> + Send,
    {
        // The Scheduler contract's lower bound: at least one unit may be in flight. A zero budget is
        // a caller bug, caught here rather than silently stalling the fan-out.
        assert!(concurrency >= 1, "concurrency must be at least one unit");
        // …and its upper bound: the budget may never exceed the engine's concurrency ceiling.
        assert!(
            concurrency <= CONCURRENCY_MAX,
            "concurrency must not exceed CONCURRENCY_MAX units"
        );

        let mut out = Vec::with_capacity(count as usize);
        for index in 0..count {
            // Serial execution keeps exactly one unit in flight at any instant; assert that it stays
            // within the budget so the in-flight <= concurrency invariant is checked, not assumed.
            let in_flight: u32 = 1;
            assert!(
                in_flight <= concurrency,
                "at most `concurrency` units may be in flight at once"
            );
            out.push(make(index).await);
        }
        assert_eq!(
            out.len(),
            count as usize,
            "the scheduler returns exactly one result per index, in order"
        );
        out
    }
}
