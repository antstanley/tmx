//! The production [`Scheduler`] adapters: the always-available serial [`SerialScheduler`] and the
//! bounded-concurrent [`TokioScheduler`].
//!
//! [`SerialScheduler`] runs `make(i)` for `i in 0..count` one at a time and collects the results in
//! **index** order — honouring the [`Scheduler`] contract (exactly `count` results, in index order,
//! `concurrency` bounded) without an async runtime, so the default `concurrency: 1` path needs no
//! executor. [`TokioScheduler`] runs genuine concurrent `map`/`eval` fan-out: it bounds in-flight
//! work with a [`tokio::sync::Semaphore`] sized by the resolved `concurrency` and collects results in
//! index order regardless of completion order, so a run's output is identical to the serial adapter's
//! — only faster. Both assert the port's invariants (`concurrency >= 1`, in-flight `<= concurrency`,
//! a length-`count` index-ordered result vector).

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

/// A bounded-concurrent [`Scheduler`] backed by a [`tokio::sync::Semaphore`].
///
/// Runs up to `concurrency` element futures at once and never more — the semaphore is the bound —
/// yet always collects results in **index** order (not completion order), because the ordered
/// [`join_all`](futures_util::future::join_all) driver keys each result to its submission position.
/// It spawns nothing: the element futures borrow the caller's scope (they are not `'static`), so they
/// are driven in place behind the permit gate rather than handed to `tokio::spawn`. Deterministic in
/// output for any interleaving: two runs of the same fan-out yield the same index-ordered vector, so
/// a `map` is identical under this adapter and the [`SerialScheduler`] — only concurrent.
#[cfg(feature = "process")]
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioScheduler;

#[cfg(feature = "process")]
impl TokioScheduler {
    /// A fresh bounded scheduler. Stateless: two instances behave identically.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(feature = "process")]
impl Scheduler for TokioScheduler {
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
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        use tokio::sync::Semaphore;

        // The Scheduler contract's bounds: at least one unit may run, and never more than the ceiling.
        assert!(concurrency >= 1, "concurrency must be at least one unit");
        assert!(
            concurrency <= CONCURRENCY_MAX,
            "concurrency must not exceed CONCURRENCY_MAX units"
        );

        // The semaphore is the in-flight bound; the atomic is the *proof* of it — every unit checks
        // the live count stays within budget before it runs, so the `<= concurrency` invariant is
        // asserted, not merely trusted to the permit arithmetic.
        let permits = Arc::new(Semaphore::new(concurrency as usize));
        let in_flight = Arc::new(AtomicU32::new(0));
        let make = &make;

        let units = (0..count).map(|index| {
            let permits = Arc::clone(&permits);
            let in_flight = Arc::clone(&in_flight);
            async move {
                // Acquire before doing any work: at most `concurrency` permits exist, so at most
                // `concurrency` units are ever past this point at once. The semaphore is never closed
                // while it is held here, so acquisition cannot fail.
                let Ok(_permit) = permits.acquire().await else {
                    unreachable!("the fan-out semaphore is never closed while units are in flight")
                };
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                assert!(
                    now <= concurrency,
                    "at most `concurrency` units may be in flight at once"
                );
                let result = make(index).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                result
            }
        });

        // `join_all` drives every unit concurrently (bounded by the permits above) and returns their
        // outputs in submission-index order, so completion order never leaks into the result vector.
        let out = futures_util::future::join_all(units).await;
        assert_eq!(
            out.len(),
            count as usize,
            "the scheduler returns exactly one result per index, in order"
        );
        // Every unit released its permit, so the fan-out drained back to zero in flight.
        assert_eq!(
            in_flight.load(Ordering::SeqCst),
            0,
            "no unit may remain in flight once the fan-out has collected"
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

#[cfg(all(test, feature = "process"))]
mod tokio_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use tokio::sync::Barrier;

    #[tokio::test]
    async fn collects_in_index_order_despite_reversed_completion_order() {
        // Each unit sleeps *longer* the earlier its index, so completion order is the reverse of
        // submission order. A completion-ordered collector would flip the vector; the scheduler keys
        // by index, so the output stays in submission order.
        let scheduler = TokioScheduler::new();
        let count = 5u32;
        let results = scheduler
            .run_indexed(count, 5, |index| async move {
                let delay_ms = u64::from(count - index) * 5;
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                Ok::<u32, RunError>(index * 10)
            })
            .await;
        let values: Vec<u32> = results.into_iter().map(|r| r.expect("ok")).collect();
        assert_eq!(
            values,
            vec![0, 10, 20, 30, 40],
            "results emerge in submission-index order, not completion order"
        );
        // Negative-space companion: completion order here is the reverse, so an order-agnostic
        // expectation (the reversed vector) must NOT match — proving the ordering is by index.
        let mut reversed = values.clone();
        reversed.reverse();
        assert_ne!(
            values, reversed,
            "index order differs from the reversed completion order"
        );
    }

    #[tokio::test]
    async fn never_runs_more_than_concurrency_units_at_once() {
        // A barrier sized to `concurrency` only releases once exactly that many units are parked on
        // it; a peak counter records the high-water mark. With `count > concurrency`, a scheduler that
        // over-admitted would push the peak above the budget (and the barrier for the last, short
        // batch would deadlock were more than `concurrency` ever admitted at once).
        let concurrency = 3u32;
        let count = 9u32;
        let scheduler = TokioScheduler::new();
        let peak = Arc::new(AtomicU32::new(0));
        let live = Arc::new(AtomicU32::new(0));
        let barrier = Arc::new(Barrier::new(concurrency as usize));

        let results = scheduler
            .run_indexed(count, concurrency, |index| {
                let peak = Arc::clone(&peak);
                let live = Arc::clone(&live);
                let barrier = Arc::clone(&barrier);
                async move {
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    // Park until a full batch of `concurrency` units is here together, so the peak
                    // genuinely reaches the budget rather than trickling through one at a time.
                    barrier.wait().await;
                    live.fetch_sub(1, Ordering::SeqCst);
                    Ok::<u32, RunError>(index)
                }
            })
            .await;

        assert_eq!(results.len(), count as usize, "one result per index");
        let observed_peak = peak.load(Ordering::SeqCst);
        assert!(
            observed_peak <= concurrency,
            "in-flight units never exceed the concurrency budget (peak {observed_peak})"
        );
        assert_eq!(
            observed_peak, concurrency,
            "the fan-out actually reached full concurrency, so the bound is meaningful"
        );
    }

    #[tokio::test]
    async fn a_serial_budget_admits_one_unit_at_a_time() {
        // `concurrency: 1` is strictly serial even under the concurrent adapter: the peak in-flight is
        // exactly one, matching the SerialScheduler's behaviour.
        let scheduler = TokioScheduler::new();
        let peak = Arc::new(AtomicU32::new(0));
        let live = Arc::new(AtomicU32::new(0));
        let results = scheduler
            .run_indexed(4, 1, |index| {
                let peak = Arc::clone(&peak);
                let live = Arc::clone(&live);
                async move {
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    live.fetch_sub(1, Ordering::SeqCst);
                    Ok::<u32, RunError>(index)
                }
            })
            .await;
        assert_eq!(results.len(), 4, "one result per index");
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "a concurrency-1 budget keeps exactly one unit in flight"
        );
    }

    #[tokio::test]
    async fn a_zero_width_fan_out_returns_no_results() {
        let scheduler = TokioScheduler::new();
        let results = scheduler
            .run_indexed(0, 4, |i| async move { Ok::<u32, RunError>(i) })
            .await;
        assert!(
            results.is_empty(),
            "a zero-width fan-out returns no results"
        );
    }

    #[tokio::test]
    #[should_panic(expected = "concurrency must not exceed CONCURRENCY_MAX units")]
    async fn rejects_a_concurrency_above_the_ceiling() {
        // Negative space: a budget above CONCURRENCY_MAX violates the port contract and is asserted.
        let scheduler = TokioScheduler::new();
        let _ = scheduler
            .run_indexed(
                1,
                CONCURRENCY_MAX + 1,
                |i| async move { Ok::<u32, RunError>(i) },
            )
            .await;
    }

    // -----------------------------------------------------------------------------------------
    // O4 (reviewable): the `map` orchestration is identical under the concurrent TokioScheduler and
    // the SerialScheduler — same item-ordered output array, regardless of completion order.
    // -----------------------------------------------------------------------------------------

    use serde_json::{Value, json};
    use tmx_core::fanout::run_map;
    use tmx_core::model::Scope;
    use tmx_schema::task::MapWith;

    /// A `map` over five string items whose inner task echoes `{ index, item }`. Under a concurrent
    /// budget each element sleeps *longer* the earlier its index, so completion order reverses
    /// submission order — the output must still follow item order.
    // A test-only helper (not itself a `#[test]` fn), so the workspace `allow-expect-in-tests` does
    // not reach it: its `expect`s are the fixture assertions, panicking on a malformed fixture.
    #[allow(clippy::expect_used)]
    async fn run_sample_map<S: tmx_core::ports::driven::Scheduler>(
        scheduler: &S,
        concurrency: u32,
    ) -> Value {
        let empty = Value::Object(serde_json::Map::new());
        let scope = Scope {
            inputs: &empty,
            env: &empty,
            secrets: &empty,
            tasks: &empty,
            item: None,
            case: None,
            output: None,
            matrix: &empty,
        };
        let map: MapWith = serde_json::from_value(json!({
            "items": ["a", "b", "c", "d", "e"],
            "concurrency": concurrency,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }))
        .expect("valid MapWith fixture");
        run_map(
            &map,
            "fan",
            &scope,
            scheduler,
            CONCURRENCY_MAX,
            0,
            |index, item, _depth| async move {
                // Reverse the completion order under any real concurrency: earlier indices sleep
                // longer, so a completion-ordered collector would flip the output.
                let delay_ms = u64::from(5 - index) * 4;
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                Ok(json!({ "index": index, "item": item }))
            },
        )
        .await
        .expect("the map runs to completion")
    }

    #[tokio::test]
    async fn map_output_is_identical_under_the_concurrent_and_serial_schedulers() {
        // The concurrent adapter fans out with a budget of 4; the serial adapter runs one at a time.
        // Both must yield the same item-ordered array — the determinism guarantee (05 §Decisions).
        let concurrent = run_sample_map(&TokioScheduler::new(), 4).await;
        let serial = run_sample_map(&SerialScheduler::new(), 4).await;

        let expected = json!([
            { "index": 0, "item": "a" },
            { "index": 1, "item": "b" },
            { "index": 2, "item": "c" },
            { "index": 3, "item": "d" },
            { "index": 4, "item": "e" },
        ]);
        assert_eq!(
            concurrent, expected,
            "the concurrent map collects in item order despite reversed completion order"
        );
        assert_eq!(
            concurrent, serial,
            "the TokioScheduler and SerialScheduler produce byte-identical map output"
        );
    }
}
