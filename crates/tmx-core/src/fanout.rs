//! `map` — the bounded fan-out orchestration over the [`Scheduler`] port.
//!
//! [`run_map`] is the *only* non-sequential construct in the engine
//! ([05 §`map`](../../../.specs/05-fan-out-and-eval.md)): it runs a single inner task once per
//! element of a resolved collection and collects the per-element outputs into an **array**, always in
//! **item order** regardless of completion order. It is pure — it owns no I/O and spawns no work of
//! its own; all concurrency crosses the injected [`Scheduler`] port, and the actual per-element task
//! execution crosses the `run_item` callback the runner supplies. The production `TokioScheduler`
//! (in `tmx-adapters`) bounds real concurrent work; the test `SerialScheduler` runs strictly
//! serially — `run_map` is identical over either.
//!
//! The algorithm mirrors 05 §`map` step for step: resolve `items` to an array and bound its width by
//! [`FANOUT_WIDTH_MAX`] (`fanout_too_wide` on excess); bound the requested `concurrency` by
//! [`CONCURRENCY_MAX`] (`concurrency_too_high` on excess) and the caller's global cap; build each
//! element's binding (the element under `item`, with a synthetic `.index`); run the inner task per
//! element through [`Scheduler::run_indexed`]; collect the results in index order (asserting the
//! output length equals the input length on *both* the producing and consuming side, Tiger-Style
//! paired assertions); and apply the element error policy — `continueOnError` records a failing
//! element's error in its slot, otherwise the first failure aborts the whole `map`.

use std::future::Future;

use serde_json::Value;
use tmx_schema::limits::{CONCURRENCY_MAX, FANOUT_WIDTH_MAX, FLOW_DEPTH_MAX};
use tmx_schema::task::{MapWith, TaskWith};

use crate::dispatch::interp_value;
use crate::error::RunError;
use crate::model::Scope;
use crate::ports::driven::Scheduler;

/// Run a `map` task's bounded fan-out and return the collected output **array** (item order).
///
/// `map` is the parsed `map` payload, `name` the task name (for typed-error attribution), `scope` the
/// parent run scope `items` is resolved against, `scheduler` the concurrency port, `concurrency_cap`
/// the run's global concurrency ceiling (the `--concurrency` flag; itself never above
/// [`CONCURRENCY_MAX`]), `depth` the current `flow`-recursion depth, and `run_item` the callback that
/// executes the inner task for one element — it receives the element's index, its `item` binding
/// (the element with a synthetic `.index` for object elements), and the depth the inner task runs at
/// (incremented when the inner task is a `flow`).
///
/// # Errors
///
/// - `fanout_too_wide` — the resolved `items` array is longer than [`FANOUT_WIDTH_MAX`] (an
///   expression over-width; a literal over-width is already rejected at preflight).
/// - `concurrency_too_high` — the requested `concurrency` exceeds [`CONCURRENCY_MAX`].
/// - `map_items_not_array` — `items` resolves to a value that is not an array.
/// - `flow_depth_exceeded` — a `flow` inner task would recurse past [`FLOW_DEPTH_MAX`].
/// - the aborting element's error — when an element fails and `continueOnError` is not set.
pub async fn run_map<S, F, Fut>(
    map: &MapWith,
    name: &str,
    scope: &Scope<'_>,
    scheduler: &S,
    concurrency_cap: u32,
    depth: u32,
    run_item: F,
) -> Result<Value, RunError>
where
    S: Scheduler,
    F: Fn(u32, Value, u32) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Value, RunError>> + Send,
{
    // 1. Resolve `items` to an array. An inline array interpolates element-wise; a lone
    //    `${{ expr }}` string resolves to the referenced value. Anything not an array is a typed
    //    error (the negative space of "iterate a collection").
    let resolved = interp_value(&map.items, scope)?;
    let items = resolved.as_array().ok_or_else(|| {
        RunError::run_failure(
            "map_items_not_array",
            format!("map task {name:?} `items` did not resolve to an array"),
        )
        .with_task(name)
    })?;
    let n = items.len();

    // Bound the fan-out width: "bounded iteration" is literally bounded (Tiger Style). An expression
    // that resolves to an over-limit array is caught here at runtime.
    if n as u64 > u64::from(FANOUT_WIDTH_MAX) {
        return Err(RunError::run_failure(
            "fanout_too_wide",
            format!(
                "map task {name:?} resolved {n} items, exceeding the {FANOUT_WIDTH_MAX} fan-out width limit"
            ),
        )
        .with_task(name));
    }
    // Backstop for the width bound now that the typed guard has passed.
    assert!(
        n as u64 <= u64::from(FANOUT_WIDTH_MAX),
        "fan-out width must be within FANOUT_WIDTH_MAX"
    );

    // 2. Resolve the concurrency budget. `concurrency` defaults to 1 (strictly in order); a request
    //    above the engine ceiling is a typed error, and the effective budget is further clamped by
    //    the run's global cap. It never drops below one unit (the Scheduler contract's lower bound).
    let requested = map.concurrency.unwrap_or(1);
    if requested > CONCURRENCY_MAX {
        return Err(RunError::validation(
            "concurrency_too_high",
            format!(
                "map task {name:?} requests concurrency {requested}, exceeding the {CONCURRENCY_MAX} ceiling"
            ),
        )
        .with_task(name));
    }
    // `CONCURRENCY_MAX >= 1` holds at compile time (a `limits` sanity assertion), so `clamp`'s
    // `min <= max` precondition is always met.
    let cap = concurrency_cap.clamp(1, CONCURRENCY_MAX);
    let effective = requested.max(1).min(cap);
    // The Scheduler contract's bounds, asserted before submit (paired with the adapter's own asserts).
    assert!(
        effective >= 1,
        "effective concurrency must be at least one unit"
    );
    assert!(
        effective <= CONCURRENCY_MAX,
        "effective concurrency must not exceed CONCURRENCY_MAX units"
    );

    // 3. A `flow` inner task consumes a recursion level (04 §Bounded flow recursion); a too-deep nest
    //    is a typed error *before* any element runs, mirroring the sequential dispatcher's guard.
    let inner_depth = if matches!(&map.task.with, TaskWith::Flow(_)) {
        if depth >= FLOW_DEPTH_MAX {
            return Err(RunError::resolution(
                "flow_depth_exceeded",
                format!(
                    "map task {name:?} flow inner task at depth {depth} would recurse past the {FLOW_DEPTH_MAX}-level bound"
                ),
            )
            .with_task(name));
        }
        depth + 1
    } else {
        depth
    };

    // 4. Run the inner task once per element through the Scheduler, collecting in INDEX order (not
    //    completion order). The Scheduler guarantees a length-`n` vector; we bind each element under
    //    `item` (with `.index`) and hand the callback the element and the inner depth.
    let run_item = &run_item;
    let results = scheduler
        .run_indexed(n as u32, effective, |index| {
            let element = bind_item(&items[index as usize], index);
            run_item(index, element, inner_depth)
        })
        .await;
    // Producing-side paired assertion: exactly one result per element, in index order.
    assert_eq!(
        results.len(),
        n,
        "the scheduler returns exactly one result per item, in index order"
    );

    // 5. Apply the element error policy while collecting the ordered output. `continueOnError` records
    //    a failing element's error in its slot (the same slot shape the sequential runner uses) and
    //    continues; otherwise the first failure aborts the whole `map`.
    let continue_on_error = map.continue_on_error.unwrap_or(false);
    let mut out: Vec<Value> = Vec::with_capacity(n);
    for result in results {
        match result {
            Ok(value) => out.push(value),
            Err(error) => {
                if continue_on_error {
                    out.push(serde_json::json!({
                        "error": serde_json::to_value(&error).unwrap_or(Value::Null),
                    }));
                } else {
                    return Err(error);
                }
            }
        }
    }
    // Consuming-side paired assertion: the output array length equals the input length.
    assert_eq!(
        out.len(),
        n,
        "the map output array holds exactly one slot per item"
    );

    Ok(Value::Array(out))
}

/// Bind one element as the inner task's `item` scope value: the element itself, plus a synthetic
/// `index` for object elements so `${{ item.index }}` reads the element's position. A scalar or array
/// element is bound unchanged (it is used whole, e.g. `${{ item }}`); an object that already defines
/// its own `index` keeps it (the element's data wins over the synthetic key).
fn bind_item(element: &Value, index: u32) -> Value {
    match element {
        Value::Object(fields) => {
            let mut bound = fields.clone();
            bound.entry("index").or_insert_with(|| Value::from(index));
            Value::Object(bound)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use serde_json::json;

    use crate::ports::driven::Scheduler;

    /// A minimal in-crate serial [`Scheduler`] for the unit tests: it runs `make(0..count)` one at a
    /// time, in index order. Defined locally rather than reusing `tmx-testkit`'s `SerialScheduler`
    /// because a `#[cfg(test)]` unit module compiled *into* `tmx-core` sees the dev-dependency's view
    /// of `tmx-core` as a distinct crate instance (the classic cyclic-dev-dep two-versions problem);
    /// the cross-adapter equivalence with the real schedulers is covered by `tmx-adapters`' tests.
    struct TestSerialScheduler;

    impl Scheduler for TestSerialScheduler {
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
            assert!(concurrency >= 1, "concurrency must be at least one unit");
            assert!(
                concurrency <= CONCURRENCY_MAX,
                "concurrency must not exceed CONCURRENCY_MAX units"
            );
            let mut out = Vec::with_capacity(count as usize);
            for index in 0..count {
                out.push(make(index).await);
            }
            assert_eq!(out.len(), count as usize, "one result per index, in order");
            out
        }
    }

    /// Drive an immediately-ready future with a no-op waker — the workspace's purity-preserving
    /// pattern (no async runtime linked into the pure core's tests). `run_map` over the serial
    /// scheduler with immediately-ready `run_item` callbacks completes on the first poll.
    fn block_on_ready<Fut: Future>(fut: Fut) -> Fut::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    /// An empty run scope — `map`'s `items` here are inline literals, so no namespace is read.
    fn empty_scope() -> (Value, Value) {
        (
            Value::Object(serde_json::Map::new()),
            Value::Object(serde_json::Map::new()),
        )
    }

    fn scope_over<'a>(empty: &'a Value) -> Scope<'a> {
        Scope {
            inputs: empty,
            env: empty,
            secrets: empty,
            tasks: empty,
            item: None,
            case: None,
            output: None,
            matrix: empty,
        }
    }

    /// Build a `MapWith` from JSON — the inner `task` is a valid (but here unexecuted) task object.
    fn map_with(value: Value) -> MapWith {
        serde_json::from_value(value).expect("valid MapWith fixture")
    }

    #[test]
    fn collects_outputs_in_item_order() {
        // Each element is echoed back through `run_item`; the collected array must follow item order.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a", "b", "c"],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out =
            block_on_ready(run_map(
                &map,
                "fan",
                &scope,
                &TestSerialScheduler,
                CONCURRENCY_MAX,
                0,
                |index, element, _depth| async move {
                    Ok(json!({ "index": index, "element": element }))
                },
            ))
            .expect("the map runs every element");
        let array = out.as_array().expect("the map output is an array");
        assert_eq!(array.len(), 3, "one output slot per item");
        assert_eq!(
            array[0],
            json!({ "index": 0, "element": "a" }),
            "the first slot is the first item's output"
        );
        assert_eq!(
            array[2],
            json!({ "index": 2, "element": "c" }),
            "the last slot is the last item's output, in item order"
        );
    }

    #[test]
    fn binds_the_element_and_a_synthetic_index_for_object_elements() {
        // An object element is bound under `item` with a synthetic `.index`; a scalar is bound whole.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": [{ "sku": "x1" }, { "sku": "x2" }],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            // `run_item` receives the already-bound `item` value; echo it so the binding is asserted.
            |_index, item, _depth| async move { Ok(item) },
        ))
        .expect("the map runs");
        let array = out.as_array().expect("array output");
        assert_eq!(
            array[0],
            json!({ "sku": "x1", "index": 0 }),
            "the first object element carries its own field and a synthetic index 0"
        );
        assert_eq!(
            array[1],
            json!({ "sku": "x2", "index": 1 }),
            "the second element's synthetic index is its position"
        );
    }

    #[test]
    fn an_over_width_expression_is_fanout_too_wide() {
        // An `items` array longer than FANOUT_WIDTH_MAX is a typed `fanout_too_wide` RunFailure.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let over = vec![Value::Null; (FANOUT_WIDTH_MAX as usize) + 1];
        let map = map_with(json!({
            "items": Value::Array(over),
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect_err("an over-width fan-out is rejected");
        assert_eq!(
            err.code, "fanout_too_wide",
            "the width error carries its code"
        );
        assert_eq!(
            err.task.as_deref(),
            Some("fan"),
            "the error names the offending map task"
        );
    }

    #[test]
    fn an_over_concurrency_request_is_rejected() {
        // A requested concurrency above CONCURRENCY_MAX is a typed `concurrency_too_high` error,
        // rejected before any element runs.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a"],
            "concurrency": CONCURRENCY_MAX + 1,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect_err("an over-concurrency request is rejected");
        assert_eq!(
            err.code, "concurrency_too_high",
            "the concurrency error carries its code"
        );
        assert_eq!(err.task.as_deref(), Some("fan"), "the error names the task");
    }

    #[test]
    fn continue_on_error_records_the_error_in_the_slot() {
        // With `continueOnError`, a failing element records its error in its own slot and the map
        // completes with a full-length output array.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["ok0", "bad1", "ok2"],
            "continueOnError": true,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |index, _element, _depth| async move {
                if index == 1 {
                    Err(RunError::run_failure("element_boom", "element one failed"))
                } else {
                    Ok(json!(index))
                }
            },
        ))
        .expect("continueOnError keeps the map running past a failing element");
        let array = out.as_array().expect("array output");
        assert_eq!(array.len(), 3, "every element still holds a slot");
        assert_eq!(array[0], json!(0), "the first element's output is recorded");
        assert_eq!(
            array[1]["error"]["code"], "element_boom",
            "the failing element's error is recorded in its own slot"
        );
        assert_eq!(array[2], json!(2), "iteration continued after the failure");
    }

    #[test]
    fn a_failing_element_aborts_the_map_without_continue_on_error() {
        // Without `continueOnError`, the first failing element aborts the whole map with its error.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["ok0", "bad1", "ok2"],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |index, _element, _depth| async move {
                if index == 1 {
                    Err(RunError::run_failure("element_boom", "element one failed"))
                } else {
                    Ok(json!(index))
                }
            },
        ))
        .expect_err("a failing element aborts the map");
        assert_eq!(
            err.code, "element_boom",
            "the abort surfaces the failing element's error"
        );
    }

    #[test]
    fn items_that_do_not_resolve_to_an_array_is_a_typed_error() {
        // Negative space: `items` resolving to a non-array value is `map_items_not_array`.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": 42,
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect_err("a non-array items is rejected");
        assert_eq!(
            err.code, "map_items_not_array",
            "the error names the non-array items"
        );
    }

    #[test]
    fn an_empty_collection_yields_an_empty_array() {
        // A zero-width fan-out is valid: it runs no elements and merges an empty array.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": [],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            0,
            |_i, _e, _d| async move { Ok(Value::Null) },
        ))
        .expect("an empty map runs");
        assert_eq!(out, json!([]), "an empty collection yields an empty array");
    }

    #[test]
    fn a_flow_inner_task_increments_the_inner_depth() {
        // A `flow` inner task runs one recursion level deeper; the callback observes depth + 1.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a"],
            "task": { "type": "flow", "with": { "use": "./sub.yaml" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            3,
            |_index, _element, depth| async move { Ok(json!(depth)) },
        ))
        .expect("the flow inner task runs");
        assert_eq!(
            out,
            json!([4]),
            "a flow inner task at map depth 3 runs its body at depth 4"
        );
    }

    #[test]
    fn a_flow_inner_task_at_the_depth_limit_is_rejected() {
        // Negative space: a `flow` inner task at FLOW_DEPTH_MAX would recurse past the bound.
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a"],
            "task": { "type": "flow", "with": { "use": "./sub.yaml" } },
        }));
        let err = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            FLOW_DEPTH_MAX,
            |_index, _element, depth| async move { Ok(json!(depth)) },
        ))
        .expect_err("a too-deep flow inner task is rejected");
        assert_eq!(
            err.code, "flow_depth_exceeded",
            "the depth guard fires before any element runs"
        );
    }

    #[test]
    fn a_leaf_inner_task_keeps_the_map_depth() {
        // A non-flow inner task does not consume a recursion level: the callback observes the map's
        // own depth unchanged (paired with the flow-increments test above).
        let (empty, _) = empty_scope();
        let scope = scope_over(&empty);
        let map = map_with(json!({
            "items": ["a", "b"],
            "task": { "type": "exec", "with": { "command": "noop" } },
        }));
        let out = block_on_ready(run_map(
            &map,
            "fan",
            &scope,
            &TestSerialScheduler,
            CONCURRENCY_MAX,
            2,
            |_index, _element, depth| async move { Ok(json!(depth)) },
        ))
        .expect("the leaf inner task runs");
        assert_eq!(
            out,
            json!([2, 2]),
            "a leaf inner task keeps the map's depth"
        );
    }
}
