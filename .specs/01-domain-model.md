# 01 — Domain Model

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

The domain model has two halves. The **input model** is the static Flow document, already defined by
[`tmx.schema.json`](../tmx.schema.json); this implementation deserialises it into Rust types in the
`tmx-schema` crate. The **runtime model** is what the engine produces — Pipeline state, task results,
scorecards, events, diagnostics — defined by [`canonical-types.schema.json`](canonical-types.schema.json)
because the data-model schema declares it out of scope ([`SCHEMA.md` decision 6](../SCHEMA.md#design-decisions-interpretations-of-the-readme)).

This page maps both halves onto Rust types. It does not re-teach the model — see the
[README](../../README.md) and [`SCHEMA.md`](../SCHEMA.md) for what each field means.

---

## ID scheme

| Id            | Type                                          | Source                                 |
| ------------- | --------------------------------------------- | -------------------------------------- |
| Run id        | `RunId` (UUIDv7, see schema)                  | `IdGenerator` port at run start        |
| Task identity | the task `name` string (unique within a Flow) | the document; also the state merge key |

`RunId` is a newtype over a 128-bit UUID, rendered lowercase-hyphenated. UUIDv7 is time-ordered, so a
lexical sort of run ids is chronological — `tmx runs list` needs no separate timestamp index. The
generator is a port so tests inject a deterministic sequence.

There is no global task id; a task is identified by its `name` within its Flow, which is also the key
its output merges under (`state[name] = output`). Every array-form task must carry an explicit,
non-empty `name` (the map form's keys supply it) — a nameless task is a `ValidationError` at
preflight ([03](03-loading-and-preflight.md#validation)). Duplicate names are a `ResolutionError`
at resolution (see [Decisions](#assumptions-and-open-questions)); the runner asserts uniqueness
before execution as a backstop (see [Invariants](04-execution-engine.md#invariants--assertions)).

---

## Input entities (the static Flow — `tmx-schema`)

These mirror the `$defs` in [`tmx.schema.json`](../tmx.schema.json) one-for-one. They derive
`serde::Deserialize`; `tmx-schema` owns them and nothing in this crate performs I/O.

### `Flow`

The only top-level document. Optional `name`, `description`, `version`, `environment`, `context`,
`inputs`; required `tasks`. `environment` and `context` are each `Inline(Box<…>) | Reference(String)`.

- `tasks: Tasks` — an ordered array **or** a name-keyed map (see [`Tasks`](#tasks)).
- `inputs: Map<String, InputSpec>` — declared inputs; each has optional `type`, `description`,
  `required`, `default`.

### `Task`

The common envelope plus a typed `with`. Fields: `name?`, `description?`, `type` (required), `if?`,
`secrets?`, `context?` + `context_strategy` + `context_precedence`, `output?`, `produces?`,
`continue_on_error`, and `with`. `with` is an enum discriminated by `type`:

```
enum TaskWith {
  Exec(ExecWith), Run(RunWith), Fetch(FetchWith), File(FileWith),
  Store(StoreWith), ChatCompletion(ChatCompletionWith), Assert(AssertWith),
  Map(Box<MapWith>), Eval(Box<EvalWith>), Flow(FlowWith),
}
```

`MapWith` and `EvalWith` are boxed because they each embed a `Task` (the inner/subject task) — a
recursive type that must not be sized inline. The boxing is also where the **bounded recursion**
guard attaches (see [04](04-execution-engine.md#bounded-flow-recursion)).

### `Tasks`

The array-or-map duality from the schema:

```
enum Tasks {
  List(Vec<Task>),                 // ordered array; runs top-to-bottom
  Map(IndexMap<String, TaskEntry>) // name-keyed; runs in source key order
}
enum TaskEntry { Task(Box<Task>), Shorthand(String) }  // Shorthand = exec(command)
```

The map form preserves **source document order** via an order-preserving map (`indexmap`), satisfying
the schema's "runs in the source document's key order" rule. A `Shorthand(String)` desugars to
`exec { command }` with the map key as the task name. This desugaring happens during resolution
(see [03](03-loading-and-preflight.md)), so the runner only ever sees a fully-formed `Task`.

### `Context`, `Environment`, `InputSpec`, supporting types

`Context` = `env`, `secrets`, `hooks` (`create`/`change`/`destroy`/`error`). `Environment` is an
**open** object (`#[serde(flatten)] extra: Map<String, Value>`) carrying provider-specific keys plus
an `options` block, mirroring the schema's `additionalProperties: true`. `Duration` is
`Seconds(u64) | Spec(String)` (e.g. `"30s"`), normalised to `Milliseconds` at resolution time.

---

## Runtime entities (what the engine produces — `tmx-core`)

These have no place in the input schema; they are defined by
[`canonical-types.schema.json`](canonical-types.schema.json) and live in `tmx-core`. They derive
`serde::Serialize` (the engine emits them) and, where seeded from disk (`--state-in`),
`Deserialize`.

| Entity          | Rust shape                             | Schema `$def`                | Description                                                                                                                                                                                                    |
| --------------- | -------------------------------------- | ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ResolvedFlow`  | struct                                 | — (internal)                 | A `Flow` after loading + reference resolution: metadata, optional resolved `environment`, resolved `context`, `inputs`, and an **ordered** `Vec<Task>` (map form sorted into key order, shorthands desugared). |
| `Pipeline`      | struct                                 | — (internal)                 | A run in flight: `id: RunId`, `state: PipelineState`, `status: RunStatus`, `results: Vec<TaskResult>`.                                                                                                         |
| `PipelineState` | `serde_json::Value` (always an object) | `PipelineState`              | The merged JSON threaded through tasks. Bounded by the state cap.                                                                                                                                              |
| `Scope`         | struct of borrowed refs                | — (internal)                 | The read-only binding environment an expression sees (see [scopes](04-execution-engine.md#state--interpolation-scopes)).                                                                                       |
| `TaskResult`    | struct                                 | `TaskResult`                 | `{ name, status, output?, error?, startedAt, ms }` for one task.                                                                                                                                               |
| `Scorecard`     | struct                                 | `Scorecard`                  | The `eval` result: `{ cases, summary, passed }`.                                                                                                                                                               |
| `Diagnostic`    | struct                                 | `Diagnostic`                 | A `validate`/`lint` finding: `{ severity, code, message, path? }`.                                                                                                                                             |
| `Event`         | enum (tagged)                          | `Event`                      | One canonical run event; see [08](08-errors-and-observability.md#events--reporters).                                                                                                                           |
| `RunRecord`     | struct                                 | `RunRecord`                  | What `RunStore` persists per run.                                                                                                                                                                              |
| `RunError`      | enum + struct                          | `RunError` / `ErrorCategory` | The typed error the core returns; see [08](08-errors-and-observability.md#error-model).                                                                                                                        |

Output normalisation (per the README): a task adapter that returns non-JSON is wrapped — valid UTF-8
text → `MessageWrapper` `{ message }`, bytes → `BlobWrapper` `{ blob: <base64> }` — before the merge,
so `PipelineState` is always JSON objects all the way down.

---

## Relationships

```
Flow 1───1 Tasks ─── 1..* Task ───0..1 TaskWith (one variant)
  │                       │
  │ 0..1                  │ map/eval embed
  ▼                       ▼
Context 0..* Hook       Task (inner / subject)   ── bounded recursion (FLOW_DEPTH_MAX)
  │ secrets                │
  ▼                        ▼ flow import
SecretSource           ResolvedFlow (another Flow)

Run time:
Pipeline 1───1 PipelineState
   │ 1
   ▼ *
TaskResult         (one per executed task, in order)
   │ when type=eval
   ▼
Scorecard 1───* EvalCase
```

A `flow`-type task, a `map`/`eval` inner task that is itself a `flow`, and a `flow`-typed provider
method all recurse into another `ResolvedFlow`. That recursion is **depth-bounded** — the one place
the otherwise-acyclic model can nest — and the bound is asserted (see
[04](04-execution-engine.md#bounded-flow-recursion)).

---

## Pipeline lifecycle (state machine)

```
            create hook
   pending ─────────────▶ running
                            │
                            │  per task: if-gate → resolve → dispatch → normalise → merge → change hook
                            │
        ┌───────────────────┼───────────────────────────┐
        │ all tasks ok      │ task failed (no            │ cancel signal
        │                   │ continueOnError)           │ (timeout / SIGINT)
        ▼                   ▼                            ▼
       ok                failed                  cancelled / timed_out
        └─────────── destroy hook (always, like `finally`) ───────────┘
```

- `create` fires once on entry to `running`. `change` fires at the end of a task **only if the merge
  changed the state** (a skipped task does not fire it) — per
  [`SCHEMA.md` resolution](../SCHEMA.md#still-open).
- `error` fires when a task aborts the Pipeline (not under `continueOnError`).
- `destroy` fires on every terminal status, success or failure — the `finally` of the lifecycle.
- Hooks run **one level deep**: a hook's tasks do not themselves fire lifecycle hooks.

Status values are `RunStatus` from the schema. The transitions are the only legal ones; the runner
asserts it never leaves a terminal status.

---

## Required read patterns (interpolation namespaces)

The runtime model is read through `${{ … }}` interpolation. These are the access patterns the
engine must support; each maps to a namespace in the [`Scope`](04-execution-engine.md#state--interpolation-scopes):

| Read                              | Namespace         | Resolves to                                                |
| --------------------------------- | ----------------- | ---------------------------------------------------------- |
| `${{ inputs.NAME }}`              | `inputs`          | a declared Flow input value                                |
| `${{ env.KEY }}`                  | `env`             | a resolved context env var                                 |
| `${{ secrets.NAME }}`             | `secrets`         | a secret the task listed in its `secrets` array; an unrequested name is not in scope (`ResolutionError`) |
| `${{ tasks.NAME.field }}`         | `tasks`           | a prior task's merged output (`PipelineState[NAME].field`) |
| `${{ item.* }}` / `${{ <as>.* }}` | `item`            | the current `map` element; `item.index` is its position    |
| `${{ case.* }}`, `${{ output }}`  | `case` / `output` | the current `eval` case and the subject's output           |
| `${{ matrix.KEY }}`               | `matrix`          | the current `--matrix` combination                         |

`tasks.NAME.field` reads typed by a task's `produces` are statically checkable; that is what `lint`
does (see [03](03-loading-and-preflight.md#lint-static-analysis-beyond-schema)).

---

## Assumptions and open questions

**Assumptions**

- `serde_json::Value` is an acceptable representation for the Pipeline state. Its dynamic, recursive
  nature is the reason allocation is bounded-not-zero (see
  [architecture-principles.md](architecture-principles.md#23-bounded-not-zero-allocation)).
- `indexmap` (or equivalent) preserves source key order for the map form of `Tasks`; the
  `SourceLoader` adapters must feed it keys in document order.

**Decisions**

- _Two-crate split for the model._ **Input types live in `tmx-schema` (deserialise-only, no I/O);
  runtime types live in `tmx-core`.** Chosen so the data model can be reused (e.g. by a future
  language-server or validator) without pulling in the execution engine, and so the runtime contract
  has a single owner.
- _Boxed recursive `with`._ **`MapWith`/`EvalWith` box their embedded `Task`.** Required for a sized
  type; it is also the natural attachment point for the depth guard.
- _Runtime types get their own schema._ **`canonical-types.schema.json` formalises the engine's
  output.** Chosen because [`SCHEMA.md` decision 6](../SCHEMA.md#design-decisions-interpretations-of-the-readme) left the Pipeline
  out of scope, leaving the event/record/scorecard shapes undocumented; consumers (CI, dashboards)
  need a contract.
- _Duplicate task names are an error, not auto-renamed._ **Two tasks with the same `name` (possible
  only in the array form; map keys are unique by construction) are a `ResolutionError` at
  resolution; the runner asserts uniqueness as a backstop.** Chosen over appending an incrementing
  suffix (`task-01`, `task-02`) because a generated name would make `${{ tasks.NAME.field }}` reads
  ambiguous, defeat `lint`'s static `produces` checking, and silently change the state keys the
  author declared; an explicit rename keeps every reference deterministic.

**Open questions**

- None currently.
