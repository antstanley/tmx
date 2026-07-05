# 02 — Crate Architecture

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

> **Read first:** [architecture-principles.md](architecture-principles.md). That page defines the
> hexagonal dependency rule, the Tiger Style tenets, and the Rust conventions. This page records the
> concrete Cargo workspace those rules produce — the crates, their module trees, and the wiring.

The implementation is a single Cargo **workspace**. Crate boundaries enforce the dependency rule
(`tmx-cli → tmx-adapters → tmx-core`) at the build level: `tmx-core` literally cannot import an
adapter because it does not depend on `tmx-adapters`.

---

## Workspace layout

```
tmx/
├── Cargo.toml                 # [workspace] members; shared lints; pinned deps
├── rust-toolchain.toml        # pinned stable toolchain + components
├── crates/
│   ├── tmx-schema/            # input data model: Flow/Task/Context/Environment (+ limits)
│   ├── tmx-core/              # pure domain core + port TRAITS + use cases
│   ├── tmx-adapters/          # one built-in adapter per driven port
│   ├── tmx-testkit/           # fake adapters (SerialScheduler, fixed Clock/IdGenerator, …)
│   └── tmx-cli/               # the `tmx` binary: driving adapter + composition root
└── tests/                     # workspace-level conformance (golden flows)
```

`docs/` (this spec, the drafts, the schemas, the examples) and `scripts/` are unchanged by the
implementation; `scripts/validate.sh` remains the schema validator until `tmx validate` reaches
parity, at which point CI may call either.

---

## Crates

### `tmx-schema` — the input data model

Deserialise-only types for the static Flow document, mirroring [`tmx.schema.json`](../tmx.schema.json)
`$defs` (see [01-domain-model.md](01-domain-model.md#input-entities-the-static-flow--tmx-schema)). Plus
the **limits** constants (`STATE_SIZE_MAX_BYTES`, `FLOW_DEPTH_MAX`, …) so both the core and the loader share
one source of truth. No I/O, no async. Depends only on `serde`, `serde_json`, `indexmap`.

```
tmx-schema/src/
├── lib.rs          # re-exports; the limits module
├── flow.rs         # Flow, Tasks, TaskEntry, InputSpec
├── task.rs         # Task envelope, TaskWith enum, the *With structs
├── context.rs      # Context, Hook, SecretSource, EnvMap
├── environment.rs  # Environment (open), Resources
├── matcher.rs      # MatcherName enum (the shared Vitest vocabulary)
├── provider.rs     # provider manifest types (tmx-provider.schema.json)
└── limits.rs       # every explicit limit constant + a compile-time sanity check
```

### `tmx-core` — the pure domain core and the ports

The hexagon's centre. Contains the execution model, the cross-cutting domain services, the **port
traits** (driving and driven), and the use cases. No `tokio`, no `std::fs`, no `std::process`, no
`std::time::SystemTime`, no `rand`. `#![forbid(unsafe_code)]`. Depends on `tmx-schema`, `serde_json`,
and trait-support crates only.

```
tmx-core/src/
├── lib.rs
├── model.rs            # runtime entities: ResolvedFlow, Pipeline, TaskResult, Scorecard, Scope …
├── error.rs            # RunError, ErrorCategory, Diagnostic
├── ports/
│   ├── mod.rs          # re-exports
│   ├── driving.rs      # RunFlow, ValidateArtifacts, LintFlow, InspectFlow, … (use-case traits)
│   └── driven.rs       # ProcessRunner, HttpClient, FileSystem, ObjectStore, ChatModel,
│                       #   SecretResolver, EnvironmentProvider, RunStore, EventSink,
│                       #   SourceLoader, ReferenceResolver, SchemaValidator, Clock,
│                       #   IdGenerator, Scheduler   (the DRIVEN port traits)
├── preflight.rs        # load → resolve → validate → capability check
├── runner.rs           # PipelineRunner: the sequential task loop
├── dispatch.rs         # TaskDispatcher: type → executor port
├── interpolate.rs      # the sandboxed ${{ }} evaluator (pure)
├── mask.rs             # the Masker (domain policy, pure)
├── matcher.rs          # the MatcherEngine (pure)
├── fanout.rs           # map + eval orchestration over the Scheduler
├── hooks.rs            # HookRunner (one level deep)
└── usecases.rs         # the use-case implementations wiring the above + driven ports
```

### `tmx-adapters` — the built-in adapters

One concrete implementation per driven port. This is where async and the heavy dependencies live:
`tokio`, `reqwest` (HttpClient), the S3-compatible SDK (ObjectStore), process spawning
(ProcessRunner), a JSON-Schema validator (SchemaValidator), `uuid` (IdGenerator). Each adapter is
behind a Cargo **feature** so a minimal or sandboxed build can drop the ones it does not need.
Depends on `tmx-core` and `tmx-schema`.

```
tmx-adapters/src/
├── lib.rs
├── process.rs      # OsProcessRunner (exec/run; language launchers)
├── http.rs         # ReqwestHttpClient
├── fs.rs           # LocalFileSystem
├── store.rs        # S3ObjectStore
├── chat.rs         # ChatCompletionsModel
├── secret.rs       # SecretResolver: env · file · provider
├── provider/       # BinaryProvider, FlowProvider
├── runstore.rs     # LocalRunStore (.tmx/runs)
├── sink/           # PrettySink (stderr), NdjsonSink (stdout), FinalStateSink (stdout)
├── loader/         # YamlLoader, JsonLoader, JsoncLoader, TomlLoader (+ kind dispatch)
├── validate.rs     # JsonSchemaValidator (Draft 2020-12)
├── clock.rs        # SystemClock
├── idgen.rs        # Uuidv7Generator
└── scheduler.rs    # TokioScheduler (bounded) — tmx-testkit provides SerialScheduler
```

### `tmx-testkit` — the fake adapters

One in-memory **fake** per driven port, mirroring `tmx-adapters` but with no real I/O: a
`SerialScheduler` (strictly serial, deterministic fan-out), a fixed `Clock` and `IdGenerator`
(frozen time, seeded UUIDv7 sequence), and recording stand-ins for `ProcessRunner`, `HttpClient`,
`ChatModel`, `FileSystem`, `ObjectStore`, `SecretResolver`, `EventSink`, and `RunStore`. The core's
unit tests, the workspace conformance `tests/`, and downstream embedders all inject these instead of
the built-in adapters — one shared, reusable fake set. Depends on `tmx-core` and `tmx-schema` only:
**no `tokio`, no `reqwest`, no I/O crate**, so it stays inside the same purity boundary as the core
it fakes (the `cargo tree` purity check covers it too).

```
tmx-testkit/src/
├── lib.rs          # re-exports; the fake-bundle constructor
├── scheduler.rs    # SerialScheduler (strictly serial, deterministic)
├── clock.rs        # FixedClock (frozen, step-advanceable)
├── idgen.rs        # SeededIdGenerator (deterministic UUIDv7 sequence)
├── process.rs      # RecordingProcessRunner (scripted stdout/exit)
├── http.rs         # FakeHttpClient (canned responses)
├── chat.rs         # FakeChatModel (canned completions)
├── fs.rs           # MemFileSystem (in-memory tree)
├── store.rs        # MemObjectStore (in-memory blobs)
├── sink.rs         # RecordingEventSink (asserts the event stream)
└── …               # SecretResolver, RunStore, loader fakes as the suite needs
```

### `tmx-cli` — the binary and composition root

The `tmx` executable: argument parsing (`clap`), the composition root that wires adapters to use
cases, the reporters' selection, and the **only** place [`ErrorCategory` → exit code](08-errors-and-observability.md#error-model)
mapping happens. Depends on `tmx-core`, `tmx-adapters`, `tmx-schema`.

```
tmx-cli/src/
├── main.rs         # parse → compose → dispatch to a use case → map error to exit code
├── compose.rs      # the composition root: config → concrete adapters → use cases
├── args.rs         # clap command/flag definitions (mirrors 07-cli.md)
├── config.rs       # layered config resolution (flags > env > project > user > system)
└── commands/       # one thin module per command, each calling a use case
```

---

## Dependency graph

```
            ┌───────────┐
            │  tmx-cli  │  clap · (anyhow only at main seam)
            └─────┬─────┘
            ┌─────▼────────┐
            │ tmx-adapters │  tokio · reqwest · s3 sdk · uuid · jsonschema
            └─────┬────────┘
            ┌─────▼─────┐
            │ tmx-core  │  serde_json · (port traits)        ◀── depends on NOTHING outward
            └─────┬─────┘
            ┌─────▼──────┐
            │ tmx-schema │  serde · indexmap
            └────────────┘

   tmx-testkit (the 5th crate) ── depends on tmx-core + tmx-schema only; no async runtime and
   no I/O crate, so it stays inside the core's purity boundary. Provides SerialScheduler, fake
   ProcessRunner/HttpClient/ChatModel, and a fixed Clock/IdGenerator — injected into tmx-core
   use cases by unit tests, the workspace conformance `tests/`, and downstream embedders.
```

The arrows are the only edges. `tmx-core`, `tmx-schema`, and `tmx-testkit` have **no async runtime,
no I/O crate** in their dependency trees — a property checked in CI (e.g. `cargo tree` assertion) so
the purity boundary cannot rot.

---

## Async model

- The **core is sync**. `PipelineRunner`, `Interpolator`, `Masker`, `MatcherEngine`, and the merge
  are ordinary functions. Where the runner must call a side-effecting port, it calls an **async
  driven-port trait method** and awaits it — so the runner's _orchestration_ methods are `async`, but
  the _pure_ services they call are not.
- **All concurrency is the `Scheduler` port's.** `map`/`eval` fan-out submits work to the Scheduler,
  which enforces the bound (`concurrency`, capped by the global `--concurrency`). No adapter spawns
  its own tasks. `tmx-testkit` supplies a `SerialScheduler` for deterministic ordering.
- **One tokio runtime**, constructed in the composition root with a bounded worker-thread count,
  drives all async adapters. The core never constructs a runtime.

---

## Composition root

`tmx-cli`'s `compose.rs` is the single wiring point:

```
fn compose(config) -> UseCases {
    // 1. construct adapters (the only place concrete types are named)
    let clock   = SystemClock::new();
    let ids     = Uuidv7Generator::new();
    let loader  = MultiFormatLoader::new();          // yaml/json/jsonc/toml + kind dispatch
    let runner  = OsProcessRunner::new(limits);
    let http    = ReqwestHttpClient::new(limits);
    // … fs, store, chat, secrets, provider, runstore, scheduler, validator …
    // 2. assemble the driven-port bundle and hand it to the use cases
    UseCases::new(Ports { clock, ids, loader, runner, http, /* … */ })
}
```

A library host writes its own `compose` (its own adapters); an HTTP host writes a third. The use
cases and the core are identical across all three — this is the embedding/sandboxing payoff
([`RUNTIME.md`](../RUNTIME.md#embedding--sandboxing)): a sandbox is just a `compose` that injects a
denying `ProcessRunner`, an allowlist `HttpClient`, and a read-only `FileSystem`.

---

## Assumptions and open questions

**Assumptions**

- Cargo features are sufficient to make adapters optional; a build with `--no-default-features` plus a
  chosen subset produces a smaller or more locked-down binary.
- A `cargo tree`-based CI check can assert that `tmx-core`/`tmx-schema` never gain an I/O or async
  dependency.

**Decisions**

- _Four production crates, not one._ **`tmx-schema` / `tmx-core` / `tmx-adapters` / `tmx-cli`** (a
  fifth, `tmx-testkit`, holds the fakes — see below). Chosen so the dependency rule is enforced by
  the compiler (the core cannot reach an adapter) and so the data model and core are reusable
  without the binary. The cost is more `Cargo.toml` boilerplate and some cross-crate trait plumbing.
- _The fakes ship as a `tmx-testkit` crate._ **The in-memory fakes — `SerialScheduler`, a fixed
  `Clock`/`IdGenerator`, recording `ProcessRunner`/`HttpClient`/`ChatModel`, and the in-memory I/O
  ports — live in a fifth workspace crate, `tmx-testkit`, depending on `tmx-core` + `tmx-schema`
  only.** Chosen so the core's unit tests, the workspace conformance suite, and downstream embedders
  inject one shared, purity-preserving fake set rather than each re-rolling its own; the cost is
  keeping the fakes in step with the port traits as they evolve.
- _Adapters behind features._ **Each driven adapter is a Cargo feature.** Chosen for minimal and
  sandboxed builds; the cost is feature-combination testing in CI.
- _Provider types in `tmx-schema`, provider adapters in `tmx-adapters`._ **The manifest is data; the
  Binary/Flow execution is an adapter.** Chosen to keep the manifest reusable by `tmx provider
validate` without the execution machinery.

**Open questions**

- _Where do the driving-port traits for a library API live?_ They are declared in
  `tmx-core::ports::driving`, but a published library facade might warrant a thin `tmx` umbrella
  crate re-exporting a curated surface. Deferred until the library host is specified.
