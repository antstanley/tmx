//! Runtime limits — the single source of truth for every bounded dimension in TMX.
//!
//! Every unbounded quantity in the engine (state size, recursion depth, task and fan-out counts,
//! expression size, captured output, the event log, the cancellation grace period, the mask-scan
//! threshold) has exactly one named constant here, so the core, the loader, and the CLI all share
//! one value ([`.specs/04-execution-engine.md` §Limits](../../../.specs/04-execution-engine.md);
//! development-guidelines.md §Limits and bounds). Reaching a limit is always an observable, typed
//! error naming the limit — never a panic or a silent truncation.
//!
//! ## Conventions
//!
//! - **Units-last, descending significance** in every identifier: `STATE_SIZE_MAX_BYTES`, not
//!   `MAX_STATE_SIZE` — the unit (`_BYTES`, `_MS`) is the suffix.
//! - **Explicit fixed-width integer types, never `usize`.** Byte sizes and durations are `u64`
//!   (they are compared against serialised lengths and can be tuned large); counts, widths, and
//!   depths are `u32`. No limit crosses a serialisation boundary as `usize`.
//! - **Config-tunable vs structurally fixed** is stated per constant. The tunable ones
//!   (`STATE_SIZE_MAX_BYTES`, `CONCURRENCY_MAX`, `EVENT_LOG_MAX_BYTES`, `CANCEL_GRACE_MS`) may be
//!   raised by a flag/env/config key; the structural ones (`FLOW_DEPTH_MAX`, `JSON_DEPTH_MAX`, and
//!   the remaining envelopes) are fixed properties of the engine.
//! - Each constant carries a **compile-time sanity assertion** (`const _: () = assert!(…)`) so a
//!   nonsensical edit fails the build rather than shipping.

/// Bytes in one mebibyte (MiB = 1024 × 1024). Named once so the size limits below read as "N MiB"
/// with no repeated magic factor; it is a unit conversion, not itself a limit.
const BYTES_PER_MIB: u64 = 1024 * 1024;

/// Maximum serialised size of the whole Pipeline state, in canonical-JSON UTF-8 bytes.
///
/// Bounds the in-memory Pipeline state threaded through the task loop. Enforced after each merge in
/// `PipelineRunner` (04 §State size cap): a merge that would exceed it aborts the run with a typed
/// `RunFailure` (`code: state_cap_exceeded`) naming the task, rather than growing without limit.
/// Tunability: **config-tunable** via `--max-state-size`, the `limits.maxStateSize` config key, or
/// `TMX_MAX_STATE_SIZE`.
pub const STATE_SIZE_MAX_BYTES: u64 = 512 * BYTES_PER_MIB;
const _: () = assert!(
    STATE_SIZE_MAX_BYTES >= 1,
    "the state cap must admit a non-empty state"
);

/// Maximum `flow`-import recursion depth.
///
/// Bounds the recursion of a `flow`-type task (and a `flow`-typed `map`/`eval` inner task or
/// provider method) back into `PipelineRunner::run`. Enforced before each recursion (04 §Bounded
/// `flow` recursion): the runner asserts `depth + 1 ≤ FLOW_DEPTH_MAX` and returns a typed
/// `ResolutionError` (`code: flow_depth_exceeded`) naming the import chain on excess. Tunability:
/// **structurally fixed** — a small limit, not a tuning knob.
pub const FLOW_DEPTH_MAX: u32 = 8;
const _: () = assert!(
    FLOW_DEPTH_MAX >= 1,
    "at least one level of flow nesting must be allowed"
);

/// Maximum number of tasks in a single Flow's task list.
///
/// Bounds the sequential task loop. Enforced at [preflight](../../../.specs/03-loading-and-preflight.md):
/// a longer task list is a typed `ValidationError` (`code: too_many_tasks`). Tunability:
/// **structurally fixed**.
pub const TASKS_PER_FLOW_MAX: u32 = 1024;
const _: () = assert!(
    TASKS_PER_FLOW_MAX >= 1,
    "a Flow must be allowed at least one task"
);

/// Maximum width of a `map`/`eval` fan-out — the length of a resolved `items`/`dataset` collection.
///
/// Bounds bounded iteration so "bounded" is literally bounded (Tiger Style). Enforced at preflight
/// for a literal array and at `items`/`dataset` resolution for an expression (05 §`map`, §`eval`):
/// a wider collection is a `ValidationError` (preflight) or `RunFailure` (runtime), both
/// `code: fanout_too_wide`. Tunability: **structurally fixed**.
pub const FANOUT_WIDTH_MAX: u32 = 100_000;
const _: () = assert!(
    FANOUT_WIDTH_MAX >= 1,
    "a fan-out must admit at least one element"
);

/// Maximum concurrent units in a single fan-out — the ceiling on any task's `concurrency` field.
///
/// Bounds the `Scheduler`'s in-flight work. Enforced at preflight (the task field and
/// `--concurrency`) and asserted at scheduler submit (04 §Limits; 05 §The Scheduler): an excess is
/// a `ValidationError` (`code: concurrency_too_high`). Tunability: **config-tunable** via
/// `--concurrency` (which itself may not exceed this ceiling).
pub const CONCURRENCY_MAX: u32 = 256;
const _: () = assert!(
    CONCURRENCY_MAX >= 1,
    "concurrency must allow at least one unit in flight"
);

/// Maximum length of a single `${{ }}` interpolation expression, in UTF-8 bytes.
///
/// Bounds the input to the sandboxed `Interpolator`. Enforced at interpolation (04 §State &
/// interpolation scopes): a longer expression is a `ResolutionError` (`code: expr_too_long`).
/// Tunability: **structurally fixed**.
pub const EXPR_LEN_MAX_BYTES: u64 = 4096;
const _: () = assert!(
    EXPR_LEN_MAX_BYTES >= 1,
    "an expression must be allowed at least one byte"
);

/// Maximum AST depth of a parsed `${{ }}` expression.
///
/// Bounds the recursion of the hand-written expression parser/evaluator. Enforced at interpolation
/// parse (04 §State & interpolation scopes): a deeper expression is a `ResolutionError`
/// (`code: expr_too_deep`). Tunability: **structurally fixed**.
pub const EXPR_DEPTH_MAX: u32 = 32;
const _: () = assert!(
    EXPR_DEPTH_MAX >= 1,
    "an expression AST must allow at least one level"
);

/// Maximum nesting depth of any parsed or merged JSON value.
///
/// Bounds recursion over JSON at parse and at state merge. Enforced at parse / merge (04 §Limits):
/// a deeper document is a `ValidationError` (`code: json_too_deep`). Tunability: **structurally
/// fixed** — a fixed property of the value model, not a tuning knob.
pub const JSON_DEPTH_MAX: u32 = 128;
const _: () = assert!(
    JSON_DEPTH_MAX >= 1,
    "JSON nesting must allow at least one level"
);

/// Maximum captured output of a single `exec`/`run`/`fetch` adapter call, in bytes.
///
/// Bounds the buffer an adapter captures from a child process or HTTP body before it becomes a task
/// output. Enforced per `exec`/`run`/`fetch` adapter (04 §Limits): a larger capture is a
/// `RunFailure` (`code: output_too_large`). Tunability: **structurally fixed**.
pub const CAPTURED_OUTPUT_MAX_BYTES: u64 = 64 * BYTES_PER_MIB;
const _: () = assert!(
    CAPTURED_OUTPUT_MAX_BYTES >= 1,
    "captured output must admit at least one byte"
);

/// Maximum number of automatic retries a `fetch` (`HttpClient`) call may attempt on a retryable
/// transport failure — the ceiling on a task's `retries` field.
///
/// Bounds the retry loop in the HTTP adapter so a flapping host can never drive unbounded requests
/// (06 §Executor ports — bounded `retries`; development-guidelines.md §Defensive coding). A task's
/// declared `retries` is clamped to this ceiling; the adapter therefore makes at most
/// `FETCH_RETRIES_MAX + 1` attempts (the initial try plus the capped retries), then returns the last
/// transport failure as a typed `RunError`. Tunability: **structurally fixed** — a safety bound, not
/// a tuning knob.
pub const FETCH_RETRIES_MAX: u32 = 5;
const _: () = assert!(
    FETCH_RETRIES_MAX >= 1,
    "the fetch retry ceiling must admit at least one retry"
);

/// Maximum number of tasks across a context's lifecycle hooks (`create`/`change`/`destroy`/`error`).
///
/// Bounds hook bodies, which run through the same runner one level deep. Enforced at preflight (04
/// §Limits): more hook tasks are a `ValidationError` (`code: too_many_hook_tasks`). Tunability:
/// **structurally fixed**.
pub const HOOK_TASKS_MAX: u32 = 256;
const _: () = assert!(HOOK_TASKS_MAX >= 1, "hooks must allow at least one task");

/// Maximum size of a run's persisted event log, in bytes.
///
/// Bounds the `RunStore`'s append-only event log. Enforced at each `RunStore` event append (04
/// §Limits): reaching it is **not an error** — it emits a `log.truncated` diagnostic and stops
/// persisting further events while the run continues. Tunability: **config-tunable**.
pub const EVENT_LOG_MAX_BYTES: u64 = 256 * BYTES_PER_MIB;
const _: () = assert!(
    EVENT_LOG_MAX_BYTES >= 1,
    "the event log must admit at least one byte"
);

/// Grace period between a cancel signal and the hard stop, in milliseconds.
///
/// Bounds how long in-flight adapters keep running after cancellation (from `--timeout` via the
/// `Clock`, or SIGINT) before the hard stop (06 §Concurrency, cancellation, timeouts; 08
/// §Cancellation, timeout, interrupt). On cancel the Scheduler stops dispatching new work and
/// in-flight adapters get this grace, then a hard stop. Tunability: **config-tunable** via
/// `--grace <dur>`.
pub const CANCEL_GRACE_MS: u64 = 5000;
const _: () = assert!(
    CANCEL_GRACE_MS >= 1,
    "the cancel grace period must be positive"
);

/// Minimum length, in bytes, at or above which a sensitive value is redacted by substring scan.
///
/// Bounds the Masker's substring redaction: values **shorter** than this are redacted on exact
/// match only, not by substring scan, so a very short secret cannot over-redact unrelated output
/// (04 §Secrets & masking; 08 §Masking at the boundary). Tunability: **structurally fixed**.
pub const MASK_SCAN_LEN_MIN_BYTES: u64 = 6;
const _: () = assert!(
    MASK_SCAN_LEN_MIN_BYTES >= 1,
    "the mask-scan floor must exclude empty values"
);

/// Default per-case pass threshold for an `eval` — the score at or above which a case counts as
/// passing when a `threshold.passScore` is not set, as a ratio in `[0, 1]`.
///
/// Colours each case's `passed` flag and defines `passRate` (05 §`eval`; 05 §Decisions:
/// "`passScore` colours cases; `threshold.metric` gates"). A `threshold.passScore`, when present,
/// overrides it. Units-last (`_RATIO`) and range-checked at compile time. Tunability: **per-flow**
/// via `threshold.passScore`; this is only the fallback default, structurally fixed.
pub const EVAL_PASS_SCORE_DEFAULT_RATIO: f64 = 0.5;
const _: () = assert!(
    EVAL_PASS_SCORE_DEFAULT_RATIO >= 0.0 && EVAL_PASS_SCORE_DEFAULT_RATIO <= 1.0,
    "the default pass score must be a ratio within [0, 1]"
);

// Cross-limit sanity relations — a mistuning that breaks one of these is nonsensical and must fail
// the build, not ship. Each states an invariant *between* limits, the negative space the per-limit
// `>= 1` checks above cannot express.

// The whole-state cap must be at least as large as a single captured output; otherwise no task's
// captured output could ever be merged into state without immediately over-capping.
const _: () = assert!(
    STATE_SIZE_MAX_BYTES >= CAPTURED_OUTPUT_MAX_BYTES,
    "the state cap must be able to hold at least one captured output"
);

// No fan-out may run more units concurrently than the widest collection it is allowed to iterate:
// the concurrency ceiling cannot exceed the fan-out width ceiling.
const _: () = assert!(
    CONCURRENCY_MAX <= FANOUT_WIDTH_MAX,
    "concurrency cannot exceed the maximum fan-out width"
);
