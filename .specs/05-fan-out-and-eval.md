# 05 — Fan-out and Evaluation

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

The two task types that run work non-linearly, plus the matcher primitive they share with `assert`.
`map` is bounded fan-out; `eval` is measurement over a dataset; both reuse one `Scheduler` for
bounded concurrency, and both collect their results in **item order** regardless of completion order.
This is the *only* non-sequential construct in TMX — the surrounding task list still runs strictly in
order (see [04](04-execution-engine.md)).

This realises [`RUNTIME.md` §map, eval, hooks](../RUNTIME.md#map-eval-hooks-produces-pure-core-orchestration).

---

## The Scheduler

`Scheduler` is a driven port (a trait) the core uses for *all* concurrency; no adapter spawns its own
tasks. It runs up to `concurrency` units at once and never more.

```
trait Scheduler {
    // Run `make(i)` for i in 0..n, at most `concurrency` concurrently;
    // return results in INDEX order (not completion order).
    async fn run_indexed<T>(&self, n: usize, concurrency: usize,
                            make: impl Fn(usize) -> Future<Output = Result<T, RunError>>)
        -> Vec<Result<T, RunError>>;
}
```

- The production adapter (`TokioScheduler`) bounds in-flight work with a semaphore; `concurrency` is
  the task's `concurrency` field, itself capped by the global `--concurrency` and `CONCURRENCY_MAX`.
- The test adapter (`SerialScheduler`) runs strictly serially for deterministic ordering.
- **Asserted invariants:** `concurrency ≥ 1` (Tiger Style: no zero-or-negative bound); in-flight count
  `≤ concurrency`; the returned vector has length `n` in index order.

`concurrency` defaults to 1 (strictly in order). There is no unbounded or distributed parallelism —
only this bounded iteration over a declared collection. Under `concurrency > 1` the interleaving of
observable side effects is **explicitly unspecified** — only the collected output order is
guaranteed; a Flow that needs ordered side effects keeps `concurrency: 1` (see
[04 Decisions](04-execution-engine.md#assumptions-and-open-questions)).

---

## `map` — bounded fan-out

`map` runs a single inner `task` (a full task object, or a `flow` import for multi-step work) once per
element of a collection, then collects the per-element outputs into an **array** under the task's
name.

| Field | Meaning |
|---|---|
| `items` | Array, or a `${{ }}` expression resolving to an array. Length bounded by `FANOUT_WIDTH_MAX`. |
| `task` | The task/`flow` run for each element. |
| `as` | Alias the current element binds under (default `item`); index as `${{ <as>.index }}`. |
| `concurrency` | Max elements at once (default 1). Output array always follows item order. |
| `continueOnError` | `true` records a failing element's error in its slot and continues; `false` aborts on the first failure. |

Algorithm:

1. Resolve `items` to an array; **assert** `len ≤ FANOUT_WIDTH_MAX` (else `RunFailure`
   `fanout_too_wide`; a literal over-limit array was already rejected at
   [preflight](03-loading-and-preflight.md#validation)).
2. Build `n` child scopes, each binding the element under `as` (with `.index`).
3. `Scheduler.run_indexed(n, concurrency, |i| run inner task in child scope i)`.
4. Collect into an ordered `Vec` — **assert output length equals `items` length** (Tiger Style
   paired assertion). On a failing element: `continueOnError` → record the error in that slot;
   else → abort the `map` (and, per the envelope, the Pipeline unless the `map` task itself sets
   `continueOnError`).
5. Merge `state[name] = [ <output per item>, … ]` (item order).

An inner `task` that is a `flow` increments the [recursion depth](04-execution-engine.md#bounded-flow-recursion).
`--matrix` CLI sugar lowers to a `map`; an authored `map` always wins (see [07](07-cli.md#matrix-sugar)).

---

## `eval` — measurement

`eval` **measures** the quality of a `subject`'s output against one or more **scorers**, optionally
over a `dataset` of cases, and emits a [`Scorecard`](canonical-types.schema.json). It is the *measure*
counterpart to `assert`'s *gate*: scores are continuous (`0..1`) and the task fails only when a
`threshold` policy is set and not met.

| Field | Meaning |
|---|---|
| `scorers` | One or more [scorers](#scorers) applied to each case's output. |
| `subject` | The task/`flow` under test, run once per case; its output is scored (`${{ output }}`). Omit to score values already in state — `${{ output }}` is then unbound and every scorer must set `actual` explicitly (`lint` checks this). |
| `dataset` | Array of case objects (or a `${{ }}`/reference resolving to one); each binds as `${{ case }}`. Omit to run once. Length bounded by `FANOUT_WIDTH_MAX`. |
| `concurrency` | Max cases at once (same bounded fan-out as `map`; default 1). |
| `threshold` | Gating policy `{ metric, min, passScore? }` — without it, `eval` only reports. |

Algorithm:

1. Resolve `dataset` (or a single synthetic case when absent); **assert** `len ≤ FANOUT_WIDTH_MAX`.
2. Via the `Scheduler`, for each case: run the `subject` once (if present), bind `${{ output }}` and
   `${{ case }}`, then apply each scorer.
3. **Per-case score** = the weighted mean of its scorers' scores. **Assert each scorer score is in
   `[0,1]`** before it is used.
4. **Aggregate** into the `summary` (`mean`, `weightedMean`, `passRate`, `min`, `p50`, `p90`,
   `count`) — every metric the `evalThreshold` enum can gate on is computed.
5. Apply the `threshold` (if any): a missed threshold is a **`RunFailure`** (exit 1) — the CLI's
   eval-as-gate behaviour. Per-case `passed` flags always compare the case score to `passScore`
   (default 0.5, taken from the threshold when one is set). Without a threshold the scorecard's
   overall `passed` is `true` and the task never fails on score.
6. Merge `state[name] = { cases, summary, passed }` ([`Scorecard`](canonical-types.schema.json)).

---

## Scorers

Each scorer yields a score in `0..1`; a case's score is the weighted mean of its scorers. Three kinds,
selected by `type`:

| `type` | Scores by | Port used |
|---|---|---|
| `matcher` (default) | a Vitest matcher → `1.0` if it passes (respecting `not`), else `0.0` | none (pure `MatcherEngine`) |
| `llmRubric` | an LLM judging the output against a rubric (model-graded) | `ChatModel` |
| `exec` / `run` | a command/script emitting a number (`{ "score": 0.9 }` or a bare number) | `ProcessRunner` |

Common to all: `name`, optional `actual` (defaults to `${{ output }}`; must be set explicitly when
the `eval` has no `subject`), `weight` (default 1, `> 0`), and an optional per-scorer `threshold`.
That per-scorer `threshold` is **advisory only** — it is carried in the schema but the eval engine
never reads it: a case's `passed` is decided solely by the case-level `passScore` (see [`eval`](#eval--measurement)),
and the run is gated solely by the task-level `threshold.metric`. An individual scorer's `threshold`
gates nothing.
The `matcher` scorer reuses the **same matcher vocabulary as `assert`** — matchers are the shared
primitive; `assert` consumes them as gates, `eval` as scorers. An `exec`/`run` scorer whose output is
not a number in `[0,1]` is a `RunFailure` (`code: scorer_bad_output`).

---

## The MatcherEngine

`MatcherEngine` is the pure core implementation of the [Vitest matcher vocabulary](../tmx.schema.json)
— the `matcherName` enum — and the single primitive behind both `assert` (gate, fail if any assertion
does not hold) and the `matcher` scorer (score `1.0`/`0.0`). It is sync, allocation-light, and has no
I/O.

- Input: an `actual` JSON value, a `MatcherName`, an optional `expected` argument (or array of
  arguments for multi-arg matchers like `toHaveProperty(path, value)`), and a `not` flag.
- Output: a boolean (pass/fail). `assert` aggregates booleans into a gate; the `matcher` scorer maps
  the boolean to `1.0`/`0.0`.
- The enum is **closed** (25 value matchers; mock/promise matchers excluded per the schema). An
  unknown matcher cannot occur — the schema rejects it at validation — but the engine asserts
  exhaustiveness so the closed set and the code cannot drift.

`assert` itself needs no adapter: it is `MatcherEngine` evaluation over interpolated `actual` values,
returning a gate result.

---

## Flow / sequence

```
map task                                   eval task
  │ resolve items (≤ FANOUT_WIDTH_MAX)        │ resolve dataset (≤ FANOUT_WIDTH_MAX)
  ▼                                            ▼
  Scheduler.run_indexed(n, concurrency) ──────┤  per case (bounded concurrency):
  │   run inner task in child scope i          │    run subject → ${{ output }}
  ▼                                            │    for each scorer: matcher | llmRubric | exec/run
  collect ordered Vec  (assert len == n)       │    case score = weighted mean (assert ∈ [0,1])
  │                                            ▼
  merge state[name] = [...]                   aggregate summary; apply threshold (miss → RunFailure)
                                               merge state[name] = { cases, summary, passed }
```

---

## Implementation layout

`tmx-core/src/fanout.rs` (`map` + `eval` orchestration over the `Scheduler` port), `matcher.rs` (the
`MatcherEngine`). The `Scheduler` trait is in `tmx-core/src/ports/driven.rs`; `TokioScheduler` is in
`tmx-adapters/src/scheduler.rs`; `SerialScheduler` is the fake in `tmx-testkit`.

---

## Assumptions and open questions

**Assumptions**

- Percentiles (`p50`/`p90`) over case scores use a defined interpolation method (e.g. nearest-rank)
  consistent across runs.
- An `llmRubric` judge returns a parseable normalised score; a non-conforming judge response is a
  `RunFailure`, not a silent zero.

**Decisions**

- *One `Scheduler` port for all concurrency.* **`map` and `eval` share it; no adapter spawns tasks.**
  Chosen per [`RUNTIME.md` decision 6](../RUNTIME.md#design-decisions): a single bound and a
  deterministic serial test mode, over ambient host threading.
- *Index-ordered collection.* **Results follow item/case order regardless of completion order.**
  Chosen because the schema defines output order; the Scheduler returns an index-ordered vector and
  the runner asserts the length.
- *Matchers are the shared primitive.* **One `MatcherEngine`/`MatcherName` enum behind `assert` and
  the `matcher` scorer.** Chosen per [`SCHEMA.md` decision 15](../SCHEMA.md#competitiveness-pass-additions)
  to avoid a parallel vocabulary; `eval` is its own task type, distinct from `assert`.
- *Fan-out width is bounded.* **`items`/`dataset` length `≤ FANOUT_WIDTH_MAX`.** Chosen so "bounded
  iteration" is literally bounded (Tiger Style); a wider collection is a typed error, not an OOM.
- *`passScore` colours cases; `threshold.metric` gates.* **Per-case `passed` always uses `passScore`
  (default 0.5); the scorecard's overall `passed` is `true` without a threshold, else "`metric ≥
  min`".** Chosen so the scorecard is self-describing whether or not a gate is set, and so
  `passRate` has a defined meaning even when the gating metric is `mean`/`weightedMean`.
- *The summary carries every gateable metric.* **`min` and `p90` are computed and emitted alongside
  `mean`/`weightedMean`/`passRate`/`p50`/`count`; the data-model schema's `evalWith` output
  description, the [README](../README.md) scorecard example, and
  [`comparison.md`](../comparison.md) are reconciled to the same list.** Chosen because the
  `evalThreshold` `metric` enum allows gating on any of them — a gateable metric the engine does
  not compute would be unimplementable.
- *Cost/latency capture is out of scope for v0.* **The scorecard `summary` does not carry token
  cost or latency for `llmRubric`/`chat-completion` scorers.** Chosen to keep the scorecard shape
  minimal; revisit with real usage.

**Open questions**

- None currently.
