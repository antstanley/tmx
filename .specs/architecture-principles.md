# Architecture Principles

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley · **Scope:** Repo-wide

The cross-cutting rules that govern how the TMX implementation is organised. Two ideas combine:
**hexagonal architecture** (the shape the [`RUNTIME.md`](../RUNTIME.md) design already implies) and
**Tiger Style** (the discipline that makes a small, auditable, sandboxable runner trustworthy). Every
per-page design decision is downstream of the rules here. Concrete crate boundaries and module trees
are in [02-crate-architecture.md](02-crate-architecture.md); concrete limit values and coding rules
are in [04-execution-engine.md](04-execution-engine.md) and
[development-guidelines.md](development-guidelines.md).

---

## 1. Hexagonal layering (ports and adapters)

The system is a hexagon: a pure domain core surrounded by adapters, every interaction with the
outside world crossing a port.

- **Domain core** — the pure execution model: sequencing, `if`, state merge, interpolation, masking,
  matchers, the scheduler abstraction, hook orchestration. No file, network, process, clock, or
  randomness access. Deterministic given its inputs and the ports it is handed.
- **Ports** — interfaces (Rust **traits**) owned by the core. A *driving* port is a use case the
  world calls (`RunFlow`, `ValidateArtifacts`, …). A *driven* port is a capability the core calls
  (`ProcessRunner`, `HttpClient`, `Clock`, …).
- **Adapters** — concrete implementations of ports. One built-in adapter per driven port; the CLI is
  one driving adapter.

### The dependency rule

**Dependencies point inward. The core depends on nothing; adapters depend on the core.** The core
imports no adapter — it is handed ports at composition time (dependency injection in the CLI's
`main`). Concretely, in Cargo terms: `tmx-cli → tmx-adapters → tmx-core`, and `tmx-core` has no
dependency on `tmx-adapters` or `tmx-cli`. A reverse edge is an architecture violation, caught in
review and (where possible) by crate boundaries.

### Why this fits TMX

TMX's defining traits *are* a ports-and-adapters decomposition (see the table in
[`RUNTIME.md`](../RUNTIME.md#why-hexagonal)): each side-effecting built-in is a driven port; the
sequential JSON-state model is the pure core; the pluggable Provider is a port the schema already
names; per-task secret masking is a domain policy enforced at the boundary. The payoff is concrete:
the whole execution model is testable with zero I/O, a Flow can run fully sandboxed by composing a
restricted adapter set, and the same core backs the CLI, a library, or an HTTP server unchanged.

---

## 2. Tiger Style tenets (as applied here)

[Tiger Style](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md) is adopted
in its **pragmatic** form (see [overview Decisions](00-overview.md#assumptions-and-open-questions)):
the bounded-everything, explicit-limit, dense-assertion core is kept in full; the static-allocation
and single-threaded tenets are relaxed where TMX's dynamic JSON state and async I/O backends require.

### 2.1 Safety through assertions

- **Assert aggressively, in positive and negative space.** Functions assert their preconditions on
  arguments and their postconditions on results. Aim for **at least two assertions per non-trivial
  function**. Assert the invariants that must hold, *and* assert that impossible states do not occur.
- **Assertions stay on in release builds.** Use `assert!` (not only `debug_assert!`) for invariants
  whose violation means corrupt state or a leaked secret. Reserve `debug_assert!` for checks that are
  expensive and provably redundant in release. An assertion failure is a controlled abort, never a
  silent miscompute.
- **Pair assertions across a boundary.** The Masker registers every resolved secret *before* any
  output port can run; an output-side assertion checks the registry is populated. The runner asserts
  `map` output length equals input length on both the producing and consuming side.
- Assertions detect **programmer** errors (broken invariants). They are not input validation —
  malformed Flows are caught by the `SchemaValidator` and returned as typed errors, never asserted.

### 2.2 Bounded everything

- **Every loop has a fixed, known upper bound.** No `loop {}` without a bound; no iteration over an
  unbounded collection without a checked limit. The task loop is bounded by `TASKS_PER_FLOW_MAX`,
  fan-out by `FANOUT_WIDTH_MAX`, recursion by `FLOW_DEPTH_MAX`.
- **Every limit is explicit, named, and asserted.** Limits are constants in `tmx-schema`, not magic
  numbers scattered in logic. Exceeding a limit is a typed error naming the limit, never a panic or
  silent truncation. The full set is the [limits table](04-execution-engine.md#limits).
- **No unbounded recursion.** The model's only recursion — `flow` import, and `flow` inner tasks of
  `map`/`eval`, and `flow`-typed provider methods — is depth-bounded and the depth is threaded
  through the call and asserted `≤ FLOW_DEPTH_MAX`.

### 2.3 Bounded, not zero, allocation

Pure Tiger Style allocates everything at startup and does zero allocation in the steady state. TMX
cannot: the Pipeline state is a dynamic, recursive `serde_json::Value` whose shape is the user's, not
ours. The adopted rule:

- **Heap allocation is permitted, but every dimension that could grow without limit is capped.** The
  serialised state size is bounded by `STATE_SIZE_MAX_BYTES` (default 512 MiB), asserted after each merge.
  Adapter outputs are bounded (`CAPTURED_OUTPUT_MAX_BYTES`). JSON nesting is bounded (`JSON_DEPTH_MAX`).
  Interpolation expressions are bounded (`EXPR_LEN_MAX_BYTES`, `EXPR_DEPTH_MAX`).
- **Hot, fixed-shape paths avoid allocation.** Interpolation, masking scans, and matcher evaluation
  operate over borrowed data and pre-sized buffers; they do not allocate per task where it can be
  avoided. The dynamic cost is paid at the state-merge boundary, which is already bounded.
- This is the single deliberate divergence from Tiger Style, recorded as a Decision below. It trades
  "zero allocation" for "bounded allocation with a hard, visible, asserted cap" — which preserves the
  property that actually matters here: a Flow cannot exhaust host memory without tripping a named
  limit first.

### 2.4 Simplicity and zero technical debt

- **Do it right the first time.** No "TODO: fix later" in merged code; no speculative generality. A
  capability that is not specified is not built (the task enum is closed; no plugin port).
- **Small functions, single responsibility.** Target ≤ 70 lines per function; a function that needs
  more is usually two. See [development-guidelines.md](development-guidelines.md).
- **Napkin math up front.** Sizes and counts are reasoned about before coding (the 512 MiB cap, the
  fan-out width) — limits are chosen, not discovered in production.

### 2.5 Determinism and testability

- The entire core is deterministic given its inputs and ports. `Clock` and `IdGenerator` are ports,
  so time and ids are injectable. The `Scheduler` test adapter runs serially.
- **Golden Flows**: a conformance suite drives `RunFlow` with recorded adapters and asserts the event
  stream + final state. This is the "spec + conformance" asset from
  [`comparison.md` §9.5](../comparison.md#9-where-tmx-is-the-clear-choice).

---

## 3. Rust conventions

- **Edition 2024**, stable toolchain pinned via `rust-toolchain.toml`. MSRV declared and tested.
- **Errors are typed, never stringly.** The core returns a `RunError` enum carrying an
  [`ErrorCategory`](canonical-types.schema.json); the CLI adapter is the only code that maps a
  category to an exit code. `thiserror` for the error enums; **`anyhow` is not used in the core or
  adapters** (it erases the category the exit-code mapping depends on). The CLI may use `anyhow` only
  at the outermost `main` seam, after the category has been extracted.
- **No `unwrap()` / `expect()` / `panic!` in non-test code**, except an asserted-impossible case with
  an explanatory message — and that case is an `assert!`, which *is* the controlled abort. Lints
  enforce this (`clippy::unwrap_used`, `clippy::expect_used` denied outside tests).
- **No `unsafe`.** `#![forbid(unsafe_code)]` in every crate. TMX has no need for it; forbidding it
  keeps the trust surface at the dependency boundary only.
- **`#[must_use]` on results and ports.** Ignoring a `Result` or a port handle is a lint error.
- **Ports are traits; the core is generic over them or holds `dyn` trait objects.** Async ports use
  native `async fn` in traits (edition 2024) where object safety allows, else `#[async_trait]`. The
  pure core (Interpolator, Masker, MatcherEngine, merge) is **sync**; only the adapter boundary is
  async.
- **Concurrency is bounded and owned by the `Scheduler` port.** No adapter spawns ambient tasks; all
  fan-out goes through the Scheduler so the bound and the deterministic test mode hold.

---

## 4. Composition root

The only place adapters are wired to ports is the CLI binary's startup (`tmx-cli`'s `main` →
`compose`). It reads configuration, constructs the concrete adapters, injects them into the use
cases, and runs the requested command. Nothing below the composition root knows which adapter is in
play. A library or HTTP-server host would have its own composition root and reuse every use case and
the core unchanged. See [02-crate-architecture.md](02-crate-architecture.md#composition-root).

---

## Assumptions and open questions

**Assumptions**

- Edition 2024's `async fn` in traits is sufficient for the driven ports; where object safety forces
  it, `#[async_trait]` is an acceptable fallback.
- A single `tokio` multi-threaded runtime, with worker threads bounded, is an acceptable host for the
  async adapters; the core's determinism does not depend on tokio's scheduling because all fan-out is
  funnelled through the `Scheduler` port.

**Decisions**

- *Pragmatic Tiger Style; bounded-not-zero allocation.* **The bounded/assertion/limit core is kept;
  static allocation is relaxed to a hard, asserted size cap on dynamic state.** Chosen because TMX's
  identity is dynamic JSON dataflow; a fixed-capacity representation would change the product, not
  just the implementation. The property preserved is the one that matters: no unbounded growth.
- *Typed errors, no `anyhow` in core/adapters.* **`RunError` + `ErrorCategory` carry the category the
  exit-code mapping needs; `anyhow` only at the CLI's outer seam.** Chosen so non-CLI hosts (HTTP,
  library) map the same categories their own way — erasing them at the source would break that.
- *`#![forbid(unsafe_code)]` everywhere.* **No crate uses `unsafe`.** Chosen because TMX needs none,
  and a hard forbid keeps audits focused on dependencies, not first-party code.
- *Assertions on in release.* **Invariant checks use `assert!`, not `debug_assert!`.** Chosen because
  a corrupt-state or leaked-secret bug must abort, not silently continue, even in production.

**Open questions**

- *Native `async fn` in traits vs `#[async_trait]`.* Edition 2024 narrows where the macro is needed,
  but object-safe `dyn Port` for the driven ports may still require it. Settle per-port during the
  first implementation spike.
- *`tokio` vs a smaller async runtime.* `reqwest` and the S3 SDK pull `tokio` regardless; if those
  adapters are made optional features, a lighter runtime could back a minimal build. Deferred.
