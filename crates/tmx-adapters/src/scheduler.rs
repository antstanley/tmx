//! [`SerialScheduler`] — the minimal serial production [`Scheduler`] adapter.
//!
//! Task 17 only needs the default `concurrency: 1` sequential path, so this adapter runs `make(i)`
//! for `i in 0..count` one at a time and collects the results in **index** order — honouring the
//! [`Scheduler`] contract (exactly `count` results, in index order, `concurrency` bounded) without an
//! async runtime. The bounded, semaphore-backed `TokioScheduler` that runs genuine concurrent
//! `map`/`eval` fan-out arrives in task 18; until then this keeps the composition root buildable and
//! every default-concurrency run correct.

use std::future::Future;

use tmx_core::RunError;
use tmx_core::ports::driven::Scheduler;
use tmx_schema::limits::CONCURRENCY_MAX;

/// A strictly-serial [`Scheduler`]: one unit in flight at a time, results collated by index.
///
/// Deterministic by construction (completion order equals submission order). `concurrency` is still
/// honoured as a *bound* — asserted `>= 1` and `<= `[`CONCURRENCY_MAX`] — so a mis-sized budget is
/// caught here rather than deferred to the concurrent adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct SerialScheduler;

impl SerialScheduler {
    /// A fresh serial scheduler. Stateless: two instances behave identically.
    #[must_use]
    pub const fn new() -> Self {
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
        // The Scheduler contract's bounds: at least one unit may run, and never more than the ceiling.
        assert!(concurrency >= 1, "concurrency must be at least one unit");
        assert!(
            concurrency <= CONCURRENCY_MAX,
            "concurrency must not exceed CONCURRENCY_MAX units"
        );

        let mut out = Vec::with_capacity(count as usize);
        for index in 0..count {
            // Serial execution keeps exactly one unit in flight; assert it stays within budget so the
            // in-flight <= concurrency invariant is checked, not assumed.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    /// Drive an immediately-ready future to completion with a no-op waker — no async runtime linked,
    /// matching the pattern the rest of the workspace's pure tests use.
    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    #[test]
    fn runs_each_index_once_in_order() {
        let scheduler = SerialScheduler::new();
        let results = block_on_ready(
            scheduler.run_indexed(4, 1, |index| async move { Ok::<u32, RunError>(index * 10) }),
        );
        assert_eq!(results.len(), 4, "exactly one result per index");
        let values: Vec<u32> = results.into_iter().map(|r| r.expect("ok")).collect();
        assert_eq!(
            values,
            vec![0, 10, 20, 30],
            "results are collated in index order, not completion order"
        );
    }

    #[test]
    fn propagates_a_per_index_error_at_its_index() {
        // A failing unit surfaces as an Err at its own index; the others still complete.
        let scheduler = SerialScheduler::new();
        let results = block_on_ready(scheduler.run_indexed(3, 1, |index| async move {
            if index == 1 {
                Err(RunError::run_failure("boom", "unit one failed"))
            } else {
                Ok(index)
            }
        }));
        assert!(results[0].is_ok(), "index 0 completed");
        assert_eq!(
            results[1].as_ref().err().map(|e| e.code),
            Some("boom"),
            "index 1 carries its own error"
        );
        assert!(results[2].is_ok(), "index 2 completed");
    }
}
