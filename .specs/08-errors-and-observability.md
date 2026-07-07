# 08 — Errors and Observability

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

What crosses the core's boundary on the way out: the typed error model, the canonical event stream
and its reporters, the run store, and the masking guarantee that wraps all of them. The unifying rule
is hexagonal — the core emits **typed categories and structured events**; the driving adapter decides
what they mean to the outside world (an exit code, a rendered line, a stored file).

Error categories and shapes are defined in [`canonical-types.schema.json`](canonical-types.schema.json);
the exit-code mapping is in [07-cli.md](07-cli.md#exit-codes).

---

## Error model

The core returns a typed `RunError` carrying an [`ErrorCategory`](canonical-types.schema.json), a
stable `code`, a human `message`, and optional `task`/`path` context. It knows nothing about exit
codes — that mapping is the CLI adapter's alone.

```
enum ErrorCategory { RunFailure, Validation, Resolution, Environment, Timeout, Interrupt }

struct RunError { category: ErrorCategory, code: &'static str, message: String,
                  task: Option<String>, path: Option<String> }
```

| Category | Raised when | Example `code` |
|---|---|---|
| `RunFailure` | a task aborted, an `assert` failed, an `eval` threshold missed, the state cap was exceeded | `state_cap_exceeded`, `assert_failed`, `eval_threshold_missed` |
| `Validation` | schema or `lint` failure, incl. a preflight task-validation failure | `schema_invalid`, `too_many_tasks` |
| `Resolution` | ref/flow/provider not found, a bad `${{ }}` or input, recursion too deep | `unknown_namespace`, `flow_depth_exceeded` |
| `Environment` | a provider method failed, or a preflight capability check failed | `provider_failed`, `missing_capability` |
| `Timeout` / `Interrupt` | `--timeout` exceeded / SIGINT | `timeout`, `interrupt` |

Rust discipline (Tiger Style + the [conventions](architecture-principles.md#3-rust-conventions)):

- Error enums use `thiserror`; **`anyhow` is not used in the core or adapters** (it erases the
  category). The CLI may use `anyhow` only at the outermost `main` seam, after the category is
  extracted for the exit-code mapping.
- **Every error is handled or propagated** — no ignored `Result` (`#[must_use]`, `unused_must_use`
  denied). No `unwrap()`/`expect()`/`panic!` outside tests except an asserted-impossible case, which
  is itself an `assert!`.
- A **panic is reserved for a broken invariant** (a programmer bug), never for an expected host/remote
  failure — those are typed `RunError`s. So a process abort always means a real defect.

Usage errors (exit 2) are raised by the CLI adapter itself (bad flag, unknown command); they are not a
core category, because the core never sees a malformed invocation.

---

## Events & reporters

The runner emits **one canonical event stream**; reporter adapters render it. Data goes to stdout,
progress to stderr.

| Event | When |
|---|---|
| `run.start` / `run.finish` | pipeline boundaries (`finish` carries status + total ms) |
| `task.start` / `task.finish` | around each task (`finish` carries status, ms, masked output) |
| `task.skip` | `if` evaluated falsy (`reason: "if=false"`) |
| `task.error` | a task failed (aborting, or recorded under `continueOnError`) |
| `map.item.finish` / `eval.case.finish` | per fan-out element / dataset case |
| `hook.start` / `hook.finish` | lifecycle hook execution |
| `log.truncated` | the per-run event log reached `EVENT_LOG_MAX_BYTES`; persistence stops (streaming continues) |

The `Event` shape is in [`canonical-types.schema.json`](canonical-types.schema.json). Reporters are
`EventSink` adapters:

- **pretty** — human summary to stderr (the TTY default for progress).
- **ndjson** — one event per line to stdout (CI / programmatic / LLM consumers).
- **final-state** — the merged JSON object to stdout (the default machine data).

`--format` selects the stdout reporter; stderr progress is independent. **Every event and final-state
payload passes through the Masker before emission** — the structural masking guarantee below.

**Diagnostics are not stream events.** A [`Diagnostic`](canonical-types.schema.json) (severity
`error`/`warning`/`info`) is the emit-only finding produced by `validate` and `lint` and by a
warn-mode `--check-produces` mismatch — a separate record, not one of the eleven `Event` variants.
The runner never routes a finding onto the canonical stream; the driving adapter surfaces it on the
reporter's **stderr channel and exit code** (`main` prints `<severity>: <message> [<code>]` and maps a
blocking finding to exit `3` for `lint`, or fails the task under `--check-produces=strict`). A
bare `--check-produces` (warn) validates the output but is non-blocking: the mismatch is reported on
stderr and the run continues.

---

## Masking at the boundary

Masking is a domain policy enforced at the output boundary, not by individual adapters (see
[04](04-execution-engine.md#secrets--masking)). The guarantee, restated at the observability layer:

- The Masker holds every secret value resolved during the run.
- **Every value leaving the core through an output port** — `EventSink` payloads, the final-state
  serialisation, `RunStore` writes, log lines — is redacted of any sensitive value, including within
  nested JSON.
- Tiger Style negative space: the runner asserts the Masker registry is populated before any output
  port can run, and each output port asserts it routed through the Masker. A new output port that
  forgets the Masker fails the assertion in tests — masking cannot be bypassed by adding a sink.
- Redaction is **value-based** in v0: every registered value is scanned for as a substring across
  emitted payloads. Values shorter than `MASK_SCAN_LEN_MIN_BYTES` (default 6, in
  `tmx-schema::limits`) are exempt from the substring scan — a 4-character secret would clobber
  unrelated text — but still redact on an exact-value match. Provenance-based tracking (which
  field a secret flowed into) is deferred.

So even a task that requested a secret and echoes it cannot surface it in any emitted artifact.

---

## Run store

`RunStore` persists each run to `./.tmx/runs/<uuidv7>/` — a final-state snapshot plus the ndjson event
log — as a [`RunRecord`](canonical-types.schema.json). IDs are **UUIDv7** (time-ordered →
chronological listings without a sort key, from the `IdGenerator` port). It is a **record, not a
journal**: no replay or durability semantics.

The persisted event log is bounded by `EVENT_LOG_MAX_BYTES` (default 256 MiB): on overflow the
store writes a final `log.truncated` event and stops persisting further events for that run, while
stdout streaming and the final-state snapshot continue unaffected — the record is capped, never
sampled.

- **Retention.** Records purge after a default **30 days**, applied opportunistically at the start of
  each `tmx run` and on demand via `tmx runs prune`; configurable via `runs.retention` /
  `TMX_RUNS_RETENTION` (`0`/`off` disables). `tmx run --no-store` opts out of recording entirely.
- **Queries.** `QueryRuns` backs `tmx runs list/show/state/logs/prune/rm`. Listings are chronological
  by id; `state` dumps the masked final state; `logs` replays the masked event log.
- The local-fs adapter is one implementation; a sqlite or remote `RunStore` is a drop-in behind the
  same port.

---

## Cancellation, timeout, interrupt

A cancellation token is threaded from the root into every adapter call (see
[06](06-ports-and-adapters.md#concurrency-cancellation-timeouts)). On `--timeout` (via `Clock`) or
SIGINT:

1. The `Scheduler` stops dispatching new work.
2. In-flight adapters get a grace period (`CANCEL_GRACE_MS`, default 5 000 ms; `--grace`
   overrides), then a hard stop.
3. The `destroy` hook fires (the `finally` of the lifecycle).
4. The run ends `cancelled`/`timed_out`; the CLI maps to exit `124`/`130`.

`clean`/`destroy` provider methods run best-effort even after a cancelled or failed run.

---

## Implementation layout

Error types in `tmx-core/src/error.rs`; the event enum in `model.rs`; reporters in
`tmx-adapters/src/sink/`; the run store in `tmx-adapters/src/runstore.rs`; the Masker in
`tmx-core/src/mask.rs`. The exit-code mapping is `tmx-cli/src/main.rs`.

---

## Assumptions and open questions

**Assumptions**

- The masking scan over emitted JSON is affordable at the event/final-state boundary; secret values
  are typically short, so a substring/structural redaction per emission is acceptable.
- The local filesystem is writable at `./.tmx/runs/` for the default `RunStore`; `--no-store` and a
  read-only `RunStore` adapter cover environments where it is not.

**Decisions**

- *Typed categories, mapping in the adapter.* **The core returns `ErrorCategory`; only the CLI maps to
  exit codes.** Per [`RUNTIME.md` decision 5](../RUNTIME.md#design-decisions): an HTTP host maps the
  same categories to status codes.
- *Panic only on a broken invariant.* **Expected host/remote failures are typed errors; a panic means
  a defect.** Chosen so a process abort is always actionable and assertions stay meaningful in release.
- *Masking is structural, asserted at the boundary.* **No sink can bypass it; an assertion guards
  every output port.** Per [`RUNTIME.md` decision 4](../RUNTIME.md#design-decisions): trusting each
  adapter to self-censor would let one forgetful sink leak.
- *Run store is a record, not a journal.* **Final-state snapshot + ndjson log, UUIDv7-keyed, 30-day
  retention.** Per [`CLI.md` decision 7](../CLI.md#design-decisions): durability/replay is explicitly
  out of scope.
- *Value-based redaction with a scan floor.* **Substring redaction applies to registered values
  `≥ MASK_SCAN_LEN_MIN_BYTES` (default 6); shorter values redact on exact match only; provenance
  tracking is deferred.** Chosen because scanning for very short values clobbers unrelated text,
  and provenance machinery is not worth its cost in v0.
- *The event log is capped, not sampled.* **`EVENT_LOG_MAX_BYTES` (default 256 MiB) bounds the
  per-run ndjson log; on overflow the store emits a final `log.truncated` event and stops
  persisting, while stdout streaming continues.** Chosen over sampling (a sampled record silently
  lies about what happened) and over failing the run (observability overflow should not abort
  work).

**Open questions**

- None currently.
