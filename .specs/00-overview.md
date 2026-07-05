# TMX Implementation — Design Overview

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley · **Scope:** Repo-wide

> **No runtime exists yet.** TMX is an early-stage spec (see [`../../README.md`](../../README.md)).
> This `docs/specs/` set is the **formal implementation specification** for the Rust runtime and
> CLI that the data-model schema and the language-neutral design drafts imply. It commits the two
> decisions those drafts left open — **host language: Rust**, **development style: Tiger Style** —
> and renders the design as concrete crates, traits, types, limits, and algorithms. Like the
> drafts, it is written in present tense ("the engine runs a preflight") to describe the design;
> that is a description of intended behaviour, not of shipped code. See
> [Decisions](#assumptions-and-open-questions).

The TMX implementation is a single Rust workspace that turns a static **Flow** (a YAML/JSON/JSONC/TOML
document conforming to [`tmx.schema.json`](../tmx.schema.json)) into a running **Pipeline** — a JSON
object threaded through a sequence of tasks — and exposes that engine through a `tmx` command-line
binary. The codebase is a **hexagon** (ports and adapters): a pure, deterministic domain core with
every side effect pushed to a port at the edge.

This document is the entry point. Detail pages are linked from each section.

---

## Problem

The TMX data model ([`SCHEMA.md`](../SCHEMA.md)) and the design drafts ([`CLI.md`](../CLI.md),
[`RUNTIME.md`](../RUNTIME.md)) define *what* TMX is and *how* it should behave, but they are
deliberately language-neutral and describe no concrete codebase. An implementer cannot start from
them without first deciding: which crates, which trait boundaries, which error model, which limits,
which allocation and concurrency discipline.

This spec answers those questions once, so the implementation is a transcription rather than a
series of ad-hoc choices. It adopts **Tiger Style** — bounded everything, explicit limits, dense
assertions, zero technical debt — because TMX's value proposition (a small, auditable, sandboxable,
embeddable runner) is exactly what that discipline produces. The one genuine tension — Tiger Style's
preference for static allocation against TMX's dynamic JSON state — is resolved in favour of
**bounded, not zero**, allocation: heap-backed state under a hard, asserted size cap.

---

## Goals

1. Execute a Flow exactly as [`RUNTIME.md`](../RUNTIME.md) specifies: preflight, then a strictly
   sequential task loop whose only non-linear move is the bounded `map`/`eval` fan-out.
2. Keep the domain core **pure** — no file, network, process, clock, or randomness access — so the
   whole execution model is unit-testable with in-memory fakes and a Flow can run fully sandboxed.
3. Project every [`CLI.md`](../CLI.md) command onto a driving use case, with the documented
   stdout/stderr contract and exit codes.
4. Enforce **explicit, asserted limits** on every unbounded dimension (state size, fan-out width,
   recursion depth, expression size, captured output) — the Tiger Style core.
5. Make the masking guarantee structural: every value leaving the core passes a Masker, so no
   adapter can leak a secret regardless of correctness.
6. Ship a conformance basis — golden Flows driving recorded adapters, asserting the event stream and
   final state — so "spec + conformance" is a real asset, not a slogan.

## Non-goals

- **No caching or incrementality.** `map` results and `eval` scorecards recompute every run.
- **No durability, journal, or replay.** The run store is a record, not a write-ahead log.
- **No scheduling, distributed execution, or unbounded parallelism.** Concurrency is bounded by a
  Scheduler port; there is no cross-machine fan-out.
- **No general branching or DAG.** Control flow is `if` skip plus bounded `map`/`eval` iteration —
  the schema's contract, not a deviation to be fixed later.
- **No plugin task types.** The task enum is closed; user extension is `flow` import (see
  [`RUNTIME.md` decision 8](../RUNTIME.md#design-decisions)).
- **No registered-name resolver.** v0 resolves references as file paths only.

---

## System shape

The codebase is a hexagon. The world drives the core through **use cases** (driving ports); the
core reaches the world only through **driven ports**, each with one built-in adapter.

```
   driving adapters          ┌──────────────────────────────┐          driven adapters
   (the world → core)        │        tmx-core (pure)        │          (core → the world)
                             │                                │
  ┌──────────┐               │  PipelineRunner   Interpolator │   ┌──▶ ProcessRunner   exec · run
  │ tmx-cli  │──┐            │  Masker  MatcherEngine          │   ├──▶ HttpClient      fetch
  └──────────┘  │  use cases │  Preflight  StateMerge          │ p ├──▶ FileSystem      file
  ┌──────────┐  ├──RunFlow──▶│  HookRunner                     │ o ├──▶ ObjectStore     store
  │ library  │──┤  Validate  │                                 │ r ├──▶ ChatModel       chat-completion
  └──────────┘  │  Lint …    │   domain entities + port traits │ t ├──▶ SecretResolver  env·file·provider
  ┌──────────┐  │            │   (the core OWNS the traits;    │ s ├──▶ EnvironmentProvider  bootstrap·deploy·…
  │ HTTP srv │──┘            │    depends on no adapter)        │   ├──▶ RunStore        .tmx/runs
  └──────────┘               │                                 │   ├──▶ EventSink       reporters
                             └──────────────────────────────┘   ├──▶ SourceLoader    yaml·json·jsonc·toml
                                                                 ├──▶ ReferenceResolver  file paths (v0)
                                                                 ├──▶ SchemaValidator JSON Schema 2020-12
        Dependencies point INWARD: tmx-cli → tmx-adapters →     ├──▶ Clock           time · timeouts
        tmx-core. The core is handed ports at composition time. ├──▶ IdGenerator     UUIDv7
                                                                 └──▶ Scheduler       bounded concurrency
```

- **`tmx-core`** — the pure execution model and the port *traits*. No `tokio`, no `std::fs`, no
  `std::process`, no system clock. Depends on `serde_json` and the data-model types only.
- **`tmx-schema`** — the data-model types (`Flow`, `Task`, `Context`, `Environment`, …) deserialised
  from the JSON model, plus the limits constants.
- **`tmx-adapters`** — one built-in adapter per driven port; this is where `tokio`, `reqwest`, the
  S3 SDK, and process spawning live.
- **`tmx-cli`** — the `tmx` binary: the driving adapter, the composition root, and the only place
  that maps core error categories to process exit codes.

See [02-crate-architecture.md](02-crate-architecture.md) for the concrete workspace.

---

## Detail pages

| Page | Topic |
|---|---|
| [01-domain-model.md](01-domain-model.md) | Data model + runtime entities as Rust types; IDs; the Pipeline lifecycle |
| [02-crate-architecture.md](02-crate-architecture.md) | Cargo workspace, crate boundaries, the dependency rule, async model, composition root |
| [03-loading-and-preflight.md](03-loading-and-preflight.md) | SourceLoader, `kind`-dispatch, reference resolution, directory assembly, the fail-fast preflight |
| [04-execution-engine.md](04-execution-engine.md) | The `PipelineRunner` algorithm, scopes & interpolation, secrets & masking, hooks, `produces`, the state cap, bounded `flow` recursion |
| [05-fan-out-and-eval.md](05-fan-out-and-eval.md) | `map` bounded fan-out, `eval` measurement, the shared MatcherEngine, scorers, the Scheduler |
| [06-ports-and-adapters.md](06-ports-and-adapters.md) | Every driven port as a trait + its built-in adapter; the TaskDispatcher; provider execution |
| [07-cli.md](07-cli.md) | The `tmx` command surface, flags, output contract, configuration, exit codes |
| [08-errors-and-observability.md](08-errors-and-observability.md) | Error categories → exit codes, the event stream, reporters, the run store, masking at the boundary |
| [architecture-principles.md](architecture-principles.md) | Hexagonal layering + Tiger Style tenets + Rust conventions (cross-cutting) |
| [development-guidelines.md](development-guidelines.md) | Tiger Style for Rust: toolchain, code style, limits meta-rule, testing, definition of done |
| [canonical-types.schema.json](canonical-types.schema.json) | JSON Schema for the **runtime/output** types the data-model schema leaves out of scope |

---

## Scope summary

| Area | Implementation | Notes |
|---|---|---|
| Data model | All of [`tmx.schema.json`](../tmx.schema.json) 0.2.0 | 10 task types, context, environment, inputs, `produces` |
| Source formats | YAML · JSON · JSONC · TOML | One `SourceLoader` port, one adapter per format |
| Execution | Sequential loop + bounded `map`/`eval` fan-out | No DAG, no branching beyond `if`, no unbounded parallelism |
| Environment providers | `binary` + `flow` adapters | Reference resolution is file-path only in v0 |
| Reference resolution | File paths | Registered-name registry out of scope in v0 |
| Run store | Local `./.tmx/runs/<uuidv7>/` | Record, not journal; 30-day default retention |
| Concurrency | Bounded via `Scheduler` port | tokio at the edge; deterministic serial test adapter |
| Allocation | Heap state under a hard, asserted size cap | Bounded, not zero — the deliberate Tiger Style deviation |

---

## Assumptions and open questions

**Assumptions**

- A POSIX-like host with a shell, filesystem, and outbound network for the default adapters; the
  core itself assumes none of these (it depends only on the ports it is handed).
- The data-model schemas ([`tmx.schema.json`](../tmx.schema.json),
  [`tmx-provider.schema.json`](../tmx-provider.schema.json)) at spec version **0.2.0** are the input
  contract; this implementation spec does not change them.
- The reader of these pages knows the TMX model (Flow / Pipeline / Task / Context / Environment /
  Provider) from the [README](../../README.md); the specs do not re-teach it.

**Decisions**

- *Spec describes intended implementation, present tense.* **The repo has no runtime; these pages
  describe the design the Rust implementation will follow, in present tense, with this status
  banner.** This matches the existing [`CLI.md`](../CLI.md)/[`RUNTIME.md`](../RUNTIME.md) drafts,
  which do the same. A future "Implemented" status flip happens per page as code lands; divergence
  between a page and the code is then a real defect to flag, per the spec discipline.
- *Specs are canonical; drafts are rationale.* **`docs/specs/` is the authoritative Rust + Tiger
  Style blueprint; [`CLI.md`](../CLI.md)/[`RUNTIME.md`](../RUNTIME.md)/[`SCHEMA.md`](../SCHEMA.md)
  are kept as the language-neutral design rationale (the "why").** The specs state the Rust "what"
  and link back to the drafts rather than restating their prose.
- *Pragmatic Tiger Style.* **tokio async only at the adapter edge; a pure sync core; bounded
  concurrency via a `Scheduler` port; heap JSON state under a hard, asserted cap; assertions stay on
  in release builds.** Chosen over a TigerBeetle-faithful (single-threaded, fixed-capacity,
  zero-allocation) reading because TMX's identity is dynamic JSON dataflow over async I/O backends
  (`reqwest`, the S3 SDK, process spawning); the bounded-everything, explicit-limit, dense-assertion
  core of Tiger Style is kept in full. See [architecture-principles.md](architecture-principles.md).

- *Library and HTTP hosts are specified after the CLI ships.* **v0 specifies only the CLI driving
  adapter; the library and HTTP-server hosts are specified once the CLI is implemented.** The use
  cases and composition root are already shaped for all three (see
  [02-crate-architecture.md](02-crate-architecture.md)), so nothing in v0 forecloses them.

**Open questions**

- *Concrete limit values.* The [limits table](04-execution-engine.md#limits) fixes defaults; several
  (max tasks per flow, max fan-out width) are first-pass envelopes, tunable once real Flows exist.
