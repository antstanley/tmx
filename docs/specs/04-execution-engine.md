# 04 — Execution Engine

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

The `PipelineRunner` is the heart of the engine: the sequential task loop that turns a `ResolvedFlow`
into a final Pipeline state. It runs **after a passing [preflight](03-loading-and-preflight.md)** and
is **sequential** (the schema's contract); the only non-linear move is the `map`/`eval` bounded
fan-out, which is handled *within* a single task and specified separately in
[05-fan-out-and-eval.md](05-fan-out-and-eval.md). This page covers the loop, the scope/interpolation
model, secrets and masking, hooks, the `produces` check, the state-size cap, bounded recursion, and
the limits and invariants that make the engine Tiger-Style-safe.

This realises [`RUNTIME.md` §Pipeline execution](../RUNTIME.md#pipeline-execution-algorithm).

---

## Responsibilities

1. Create the Pipeline state and fire the `create` hook.
2. For each task in order: gate on `if`, resolve context, interpolate `with`, dispatch, normalise the
   output, optionally check `produces`, merge, fire `change`.
3. Enforce error policy (`continueOnError` vs abort) and the state-size cap.
4. Fire the `destroy` hook on every terminal status; return the final state and per-task results.

The runner is `async` (it awaits driven ports) but holds **no I/O of its own**; every effect is a port
call. It is generic over the port bundle so tests inject fakes.

---

## Pipeline execution algorithm

`PipelineRunner::run(flow, inputs, ports, depth) -> Result<Pipeline, RunError>`:

1. **Create state.** `state = {}` (an empty JSON object). Fire the context `create` hook. Emit
   `run.start`.
2. **For each task, in order** (loop bounded by `TASKS_PER_FLOW_MAX`):
   1. **`if` gate.** Evaluate `task.if` against the current [scope](#state--interpolation-scopes).
      Falsy → emit `task.skip` (`reason: "if=false"`), leave state unchanged, **do not** fire
      `change`. Continue to the next task.
   2. **Resolve context.** Merge the inherited (folder/Flow) context with the task's `context` per
      `contextStrategy` (`merge`/`replace`) and `contextPrecedence` (`local`/`inherited`), each
      section (`env`/`secrets`/`hooks`) independently.
   3. **Resolve `with`.** Interpolate every `${{ }}` in the task config against the scope. Secret
      references resolve to **unmasked** values **only** for the names in `task.secrets`; every
      resolved secret value is registered with the [Masker](#secrets--masking) as sensitive.
   4. **Dispatch.** Route by `type` through the [TaskDispatcher](06-ports-and-adapters.md#taskdispatcher)
      to the executor port; record timing via the `Clock` port. Emit `task.start`/`task.finish`.
   5. **Normalise output.** Non-JSON adapter result → valid UTF-8 text becomes `{ "message": … }`;
      bytes become `{ "blob": <base64> }`.
   6. **Conformance (optional).** If `task.produces` is set **and** runtime checking is enabled,
      validate the output via `SchemaValidator`; on mismatch emit a `Diagnostic` (warn by default,
      fail under `--check-produces=strict`).
   7. **Merge.** `state[task.output ?? task.name] = output`. **Assert** the serialised state size
      `≤ STATE_SIZE_MAX_BYTES` (see [state cap](#state-size-cap)); over-cap → `RunFailure` naming the task.
   8. **`change` hook.** If the merge changed the state, fire `change` — once per state-changing task,
      **one level deep** (a `change` hook that mutates state does not re-trigger `change`).
   9. **Errors.** If the task failed and `continueOnError` (task **or** global) is set → record the
      error in the task's state slot, emit `task.error`, continue. Otherwise → fire the `error` hook,
      set `status = failed`, **stop the loop**.
3. **Finish.** Fire the `destroy` hook (always — success or failure, like `finally`). Emit
   `run.finish`. Return the final state and `Vec<TaskResult>`.

`assert`, `map`, and `flow` need **no adapter** — they are pure-core: matcher evaluation, the
[Scheduler](05-fan-out-and-eval.md)-driven fan-out, and bounded recursion into `PipelineRunner`
respectively. Hooks run their bodies through the *same* runner (so hooks inherit the full task model)
but one level deep — see [hooks](#lifecycle-hooks).

---

## State & interpolation scopes

The `Interpolator` is a **pure** function `(expression, scope) -> Result<Value, RunError>`. `${{ … }}`
expressions are a **JavaScript subset**: member access, literals, comparison with **strict** equality
(`===`/`!==`), boolean/`!` logic, and JS truthy/falsy — **no** function calls, assignment, or
arbitrary code. It is a hand-written sandboxed evaluator over a bounded AST; it never calls a JS
engine or `eval`. Expression length and AST depth are bounded (`EXPR_LEN_MAX_BYTES`, `EXPR_DEPTH_MAX`).

The `Scope` is a struct of borrowed references exposing these namespaces:

| Namespace | Available where | Source |
|---|---|---|
| `inputs.*` | everywhere | declared Flow `inputs` (CLI `--input`, calling `flow` task, defaults) |
| `env.*` | everywhere | resolved context `env` |
| `secrets.*` | everywhere (values masked unless the task opted in) | resolved context `secrets` |
| `tasks.*` | everywhere | the Pipeline `state` — `tasks.NAME.field` reads a prior task's output |
| `item.*` (or the `as` alias) | inside a `map` inner task | the current element; `item.index` is the zero-based index |
| `case.*`, `output` | inside `eval` scorers/subject | the current dataset case; `output` is the subject's output |
| `matrix.*` | when run via `--matrix` sugar | the current matrix combination |

A resolution failure (unknown namespace key, type mismatch against a declared input) is a
`ResolutionError` (exit 4). `lint` catches the statically-checkable ones — including `produces`-typed
`tasks.NAME.field` references — before a run.

---

## Secrets & masking

Masking is a **domain policy** enforced at the port boundary, so no adapter can leak a secret
regardless of its own correctness:

- The `Masker` holds a registry of **sensitive values** — every secret value resolved during a run.
- A task receives a secret **unmasked** (so it can use it) only if it lists the name in its `secrets`
  array; unrequested secrets are never resolved into that task's scope at all.
- **Every value leaving the core through an output port** — `EventSink` payloads, the final-state
  serialisation to stdout, `RunStore` writes, log lines — passes through the Masker, which redacts
  occurrences of any sensitive value (including within nested JSON). So even a task that *did* request
  a secret and echoes it cannot surface it.

This is defence-in-depth on the opt-in model: the schema decides *who gets* a secret; the Masker
guarantees it *never appears* in anything emitted. The paired assertion (Tiger Style negative space):
the runner asserts the Masker registry contains every resolved secret **before** any output port can
run, and every output port asserts it routed through the Masker.

---

## Lifecycle hooks

`create` / `change` / `destroy` / `error` hook bodies run through the **same** `PipelineRunner`, so a
hook inherits the full task model (it can `exec`, `fetch`, even `map`). Two rules keep them safe:

- **One level deep.** A hook's tasks do **not** themselves fire lifecycle hooks. A `change` hook that
  mutates state does not re-trigger `change`. This makes hooks terminating and predictable (no
  hook-storm) and is asserted: the runner refuses to fire a lifecycle hook when already inside one.
- **`change` fires once per state-changing task**, and only when the state actually changed (a skipped
  task does not fire it), per [`SCHEMA.md` resolution](../SCHEMA.md#still-open).

`destroy` runs on every terminal status (success, failure, cancellation) — the `finally` of the
lifecycle — including after a failed provider teardown.

---

## `produces` conformance

`produces` is **declarative**; it never affects execution. Runtime conformance checking is **opt-in**
(off by default, per the schema): with `--check-produces=strict` the runner validates each task's
output against its `produces` schema and fails on mismatch; otherwise a mismatch is a warning
`Diagnostic`. `lint` always uses `produces` statically regardless of the runtime flag.

---

## Bounded `flow` recursion

A `flow`-type task recurses into `PipelineRunner::run`; so can a `map`/`eval` inner task that is a
`flow`, and a `flow`-typed provider method. Tiger Style forbids unbounded recursion, so the depth is
**explicit, threaded, and asserted**:

- The runner carries a `depth` parameter, starting at 0. Each recursion increments it.
- Before recursing, the runner **asserts `depth + 1 ≤ FLOW_DEPTH_MAX`**; exceeding it is a
  `ResolutionError` (`code: flow_depth_exceeded`) naming the import chain.
- This is a second backstop to the loader's [cycle detection](03-loading-and-preflight.md#reference-resolution):
  cycles are caught structurally at resolution; depth is bounded at execution even for acyclic but
  pathologically deep nests.

The bound (default 8) is small and named; it is a limit, not a tuning knob, and lives in the
[limits table](#limits).

---

## State size cap

The whole Pipeline state is held **in memory** and threaded through tasks. To keep that bounded, the
serialised state has a default cap of **512 MiB** (`STATE_SIZE_MAX_BYTES`). A merge that would exceed it
**aborts the run** (`RunFailure`, `code: state_cap_exceeded`) naming the offending task, rather than
growing without limit. `{ "blob": … }` outputs and large `map`/`eval` result arrays count toward the
cap. The cap is raised via `--max-state-size`, the `limits.maxStateSize` config key, or
`TMX_MAX_STATE_SIZE`. There is **no spill-to-disk / external `Blob` port** in v0 — an explicit,
visible, asserted limit is preferred over silent external storage.

---

## Limits

Every unbounded dimension has an explicit, named constant in `tmx-schema::limits`. They are checked at
[preflight](03-loading-and-preflight.md#validation) where possible, and asserted at the point of use.
Exceeding a limit is always a typed error naming the limit — never a panic or silent truncation.

| Constant | Default | Enforced | Error |
|---|---|---|---|
| `STATE_SIZE_MAX_BYTES` | 512 MiB | after each merge | `RunFailure` `state_cap_exceeded` |
| `FLOW_DEPTH_MAX` | 8 | before each `flow` recursion | `ResolutionError` `flow_depth_exceeded` |
| `TASKS_PER_FLOW_MAX` | 1024 | preflight | `ValidationError` `too_many_tasks` |
| `FANOUT_WIDTH_MAX` | 100 000 | preflight / at `items` resolution | `RunFailure` `fanout_too_wide` |
| `CONCURRENCY_MAX` | 256 | at scheduler submit | `ValidationError` `concurrency_too_high` |
| `EXPR_LEN_MAX_BYTES` | 4 096 bytes | at interpolation | `ResolutionError` `expr_too_long` |
| `EXPR_DEPTH_MAX` | 32 | at interpolation parse | `ResolutionError` `expr_too_deep` |
| `JSON_DEPTH_MAX` | 128 | at parse / merge | `ValidationError` `json_too_deep` |
| `CAPTURED_OUTPUT_MAX_BYTES` | 64 MiB | per `exec`/`run`/`fetch` adapter | `RunFailure` `output_too_large` |
| `HOOK_TASKS_MAX` | 256 | preflight | `ValidationError` `too_many_hook_tasks` |

Concrete values are first-pass envelopes (the *rule* "declare every limit" is global
[development-guidelines.md](development-guidelines.md); the *values* are this implementation's
concern). They are tunable via config where it makes sense (`STATE_SIZE_MAX_BYTES`, `CONCURRENCY_MAX`); the
structural ones (`FLOW_DEPTH_MAX`, `JSON_DEPTH_MAX`) are fixed.

---

## Invariants & assertions

Tiger Style: assert preconditions and postconditions, positive and negative space, in release. The
runner's load-bearing invariants:

- **State is always a JSON object** at the top level — asserted on entry and after every merge.
- **Merge key is non-empty**; the resolved `output ?? name` is a non-empty string — asserted before
  merge.
- **Task index is in `[0, n)`** for the bounded loop — asserted each iteration.
- **State size `≤ STATE_SIZE_MAX_BYTES`** after every merge — asserted (and returned as a typed error, not
  only asserted, since it can be triggered by input).
- **`map` output length equals input length** — asserted on both producing and consuming sides
  ([05](05-fan-out-and-eval.md)).
- **Eval scores are in `[0,1]`** — asserted as each scorer returns.
- **The Masker registry holds every resolved secret before any output port runs** — asserted at the
  output boundary (negative space: nothing leaves unmasked).
- **`depth ≤ FLOW_DEPTH_MAX`** at every recursion — asserted.
- **No lifecycle hook fires inside a hook** — asserted (one-level guarantee).
- **The Pipeline never leaves a terminal status** — asserted at finish.

The distinction matters: invariants broken by a **programmer** bug are `assert!` (controlled abort);
conditions reachable by **malformed input** (over-cap state, too-wide fan-out) are typed errors *and*
optionally asserted as a backstop.

---

## Implementation layout

`tmx-core/src/runner.rs` (the loop), `interpolate.rs` (the evaluator), `mask.rs` (the Masker),
`hooks.rs` (the HookRunner), `dispatch.rs` (the type→port seam). The fan-out paths live in
`fanout.rs` ([05](05-fan-out-and-eval.md)). Limits are `tmx-schema::limits`.

---

## Assumptions and open questions

**Assumptions**

- Serialised-size measurement of `serde_json::Value` is cheap enough to run after every merge (an
  incremental size accounting, not a full re-serialise each time).
- A hand-written expression evaluator covering the JS subset is tractable and small; no third-party JS
  engine is embedded.

**Decisions**

- *Bounded recursion for `flow`, not an explicit work-stack.* **Recursion into `PipelineRunner` with
  a threaded, asserted `depth ≤ FLOW_DEPTH_MAX`.** Chosen over reifying an explicit stack because the
  depth bound is small and the recursion mirrors the model's natural shape; Tiger Style's concern
  (unbounded growth) is met by the asserted bound.
- *State cap is a typed error and an assertion.* **Over-cap is `RunFailure` (input-reachable) and also
  asserted (backstop).** Chosen so a real workload gets a clean error while a logic bug still aborts.
- *Hand-written sandboxed evaluator.* **No embedded JS engine.** Chosen for a minimal trust surface
  and determinism, at the cost of implementing the subset ourselves; the subset is deliberately tiny.
- *Masking at the output boundary, not by callers.* **The core registers sensitive values; every
  output port redacts.** Per [`RUNTIME.md` decision 4](../RUNTIME.md#design-decisions): one forgetful
  adapter would otherwise leak.

**Open questions**

- *Observable side-effect ordering under `concurrency > 1`.* Output order is defined (item order);
  should the spec also constrain the *order of observable side effects* during fan-out, or leave it
  explicitly unspecified? (Open in [`RUNTIME.md`](../RUNTIME.md#open-questions).)
- *Incremental state-size accounting.* The exact accounting (bytes of canonical JSON vs in-memory
  estimate) needs pinning so the cap is reproducible across hosts.
