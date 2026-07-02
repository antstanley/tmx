# TMX Runtime — Design Proposal

> The execution engine that turns a static [Flow](../README.md#flows) into a running
> [Pipeline](../README.md#concepts) — architected as a **hexagon** (ports & adapters).
>
> **Status: design draft (runtime v0), no implementation yet.** This document specifies the
> *engine* behind the [CLI](./CLI.md): how a Flow is loaded, resolved and executed; how state,
> interpolation, secrets, `map`, `eval` and hooks behave at runtime; and how every side-effecting
> concern is isolated behind a port so it can be swapped, sandboxed or tested. It targets **spec
> version 0.2.0** of [`tmx.schema.json`](./tmx.schema.json) and
> [`tmx-provider.schema.json`](./tmx-provider.schema.json), and backs every command in
> [`CLI.md`](./CLI.md). It is language-neutral — the ports are interfaces; Go, Rust and TypeScript
> are all viable hosts.

## Why hexagonal

[Hexagonal architecture](https://en.wikipedia.org/wiki/Hexagonal_architecture_(software))
(Alistair Cockburn's *ports & adapters*) puts a **pure domain core** at the centre and pushes every
interaction with the outside world to the edge, behind an interface (**port**) with one or more
interchangeable implementations (**adapters**). It fits TMX better than most systems, because TMX's
defining traits *are already* a ports-and-adapters decomposition waiting to be named:

| TMX trait (from the schema) | Hexagonal reading |
| --- | --- |
| **Batteries-included built-ins** (`exec`/`fetch`/`file`/`store`/`chat-completion`/…) | Each side-effecting task type is a **driven port** with a built-in **adapter** |
| **Sequential JSON-state dataflow** (`state[name] = output`) | The **domain core** — pure, deterministic, no I/O |
| **Pluggable Provider** (binary or Flow, `bootstrap`/`deploy`/`clean`/`destroy`) | The schema already *names a port* (`EnvironmentProvider`) with two adapters |
| **Per-task opt-in secret masking** | A cross-cutting **domain policy** enforced at the port boundary |
| **One model, four formats** (`kind`-dispatch) | A **SourceLoader** port; one adapter per format |
| **Embeddable, validatable task-DSL** ([`comparison.md` §9.5](./comparison.md#9-where-tmx-is-the-clear-choice)) | The use cases are **driving ports** — embed the core, bring your own adapters |

The payoff is concrete: the whole sequential-execution model is **testable with zero I/O**
(inject fake adapters), a Flow can be evaluated **fully sandboxed** (swap `ProcessRunner` for a
denying adapter), and the same core runs behind the CLI, a library API, or an HTTP server without
change.

## The hexagon

```
        DRIVING (primary) adapters                    DRIVEN (secondary) adapters
        — the world calls the core —                  — the core calls the world —

   ┌───────────────┐                                          ┌────────────────────┐
   │   tmx CLI     │─┐                                   ┌────▶│ ProcessRunner      │ exec · run
   └───────────────┘ │     ┌────────────────────────┐   │     ├────────────────────┤
   ┌───────────────┐ │     │  Application use cases  │   ├────▶│ HttpClient         │ fetch
   │  Library API  │─┼────▶│     (driving ports)     │   │     ├────────────────────┤
   └───────────────┘ │     ├────────────────────────┤   ├────▶│ FileSystem         │ file
   ┌───────────────┐ │     │      DOMAIN CORE        │   │     ├────────────────────┤
   │  HTTP server  │─┘     │  pure execution model:  │──▶│────▶│ ObjectStore        │ store
   └───────────────┘       │  Pipeline · Interpolator│   │     ├────────────────────┤
                           │  Masker · Matchers ·    │ p ├────▶│ ChatModel          │ chat-completion
                           │  Scheduler · HookRunner │ o │     ├────────────────────┤
                           └────────────────────────┘ r ├────▶│ SecretResolver     │ env·file·provider
                                                        t ├────▶│ EnvironmentProvider│ bootstrap·deploy·…
                                                        s ├────▶│ RunStore           │ .tmx/runs
                                                          ├────▶│ EventSink          │ reporters
                                                          ├────▶│ SourceLoader       │ yaml·json·jsonc·toml
                                                          ├────▶│ SchemaValidator    │ JSON Schema 2020-12
                                                          └────▶│ Clock · IdGen      │ time · UUIDv7
```

Dependencies point **inward**: adapters depend on ports, ports are owned by the core, the core
depends on nothing. The core never imports an adapter; it is handed ports at composition time
(dependency injection in the CLI's `main`).

## Domain core

The core is pure orchestration over the [schema model](./SCHEMA.md). It contains **no** file,
network, process, clock or randomness access — those arrive only through ports.

### Entities

| Entity | Description |
| --- | --- |
| `ResolvedFlow` | A Flow after loading + reference resolution: metadata, optional `environment`, resolved `context`, `inputs`, and an **ordered** `tasks` list (array form, or map form sorted into key order). |
| `Task` | The resolved task envelope (`name`, `if`, `secrets`, context overrides, `output`, `produces`, `continueOnError`) plus a typed `with`. |
| `Pipeline` | A run in flight: `id` (UUIDv7), `state` (the JSON object), `status`, and the accumulating event log. |
| `Scope` | The read-only binding environment an expression sees (see [scopes](#state--interpolation-scopes)). |
| `TaskResult` | `{ output, status, error?, startedAt, ms }` for one task. |
| `Scorecard` | The `eval` result: `{ cases[], summary, passed }`. |
| `Diagnostic` | A `validate`/`lint` finding: `{ severity, code, message, path }`. |

### Preflight — load · validate · capability check

Before any side effect, the engine runs a **preflight** that either passes wholesale or fails fast
with nothing executed:

1. **Load & resolve.** Parse the target via `SourceLoader` and resolve references. A **directory**
   target is assembled into a flow: the sibling `environment.*` / `context.*` become the shared
   config, and every other artifact is a task. **Task files may use any filename** — identity is the
   `kind` discriminator, or (when `kind` is omitted) "not a reserved `environment`/`context`/`flow`
   artifact"; tasks order by natural filename.
2. **Validate.** Every task artifact (and the flow, environment, context) is checked against the
   schema via `SchemaValidator`. Any failure → **ValidationError** (exit `3`) — a malformed task
   aborts the whole run before the first task executes, so a folder never runs half-way.
3. **Capability check.** The engine computes the set of ports the flow will touch (from the task
   `type`s used and the `environment`'s provider) and verifies each bound adapter is **present and
   real** — not a stub or denying adapter. A flow that uses `store` requires a working `ObjectStore`;
   if one is not wired, preflight fails fast with **EnvironmentError** (exit `5`) naming the missing
   capability, rather than aborting mid-run at the first `store` task.

### Pipeline execution algorithm

`PipelineRunner.run(flow, inputs, ports)` is the heart of the engine. It runs **after a passing
[preflight](#preflight--load--validate--capability-check)** and is **sequential** (the schema's
contract); the only non-linear move is the `map`/`eval` bounded fan-out, handled *within* a single
task.

1. **Create state.** Initialise `state = {}`; fire the context `create` hook.
2. **For each task, in order:**
   1. **`if` gate.** Evaluate `task.if` against the current [scope](#state--interpolation-scopes).
      Falsy → emit `task.skip`, leave state unchanged, **do not** fire `change`. Continue.
   2. **Resolve context.** Merge inherited (folder/Flow) context with the task's `context` per
      `contextStrategy` (`merge`/`replace`) and `contextPrecedence` (`local`/`inherited`),
      per-section (`env`/`secrets`/`hooks`).
   3. **Resolve `with`.** Interpolate every `${{ }}` in the task config against the scope. Secret
      references resolve to **unmasked** values **only** for the secrets named in `task.secrets`;
      every resolved secret value is registered with the [Masker](#secrets--masking) as sensitive.
   4. **Dispatch.** Route by `type` through the [TaskDispatcher](#driven-side--ports--adapters) to
      the executor port; record timing.
   5. **Normalise output.** If the adapter returns non-JSON: valid UTF-8 text → `{ "message": … }`;
      bytes → `{ "blob": <base64> }` (per the README contract).
   6. **Conformance (optional).** If `task.produces` is set *and* `--check-produces` is enabled,
      validate the output via `SchemaValidator`; on mismatch emit a warning diagnostic under
      `warn` (the bare-flag default value), or fail the task under `strict`; with the flag absent,
      outputs are not checked at run time.
   7. **Merge.** `state[task.output ?? task.name] = output`.
   8. **`change` hook.** If the merge changed the state, fire the context `change` hook
      ([SCHEMA.md resolution](./SCHEMA.md#still-open): once per state-changing task). Hooks are
      **one level deep** — a `change` hook that mutates state does **not** re-trigger `change` (no
      recursive lifecycle-hook firing).
   9. **Errors.** If the task failed and `continueOnError` (task **or** global `--continue-on-error`)
      is set → record the error in the task's state slot, emit `task.error`, continue. Otherwise →
      fire the `error` hook, set `status = failed`, **stop**.
3. **Finish.** Fire the `destroy` hook (always — success or failure, like `finally`). Return the
   final state and per-task results.

`assert`, `map` and `flow` need **no adapter** — they are pure-core (matcher evaluation, the
scheduler, and recursion into `PipelineRunner` respectively). Hooks run their task bodies through
the *same* runner, so hooks inherit the full task model for free — but **one level deep**: a hook's
tasks do not themselves fire lifecycle hooks (no recursive `create`/`change`/`destroy`/`error`).

### State & interpolation scopes

The Interpolator is a **pure** function `(expression, scope) → value`. `${{ … }}` expressions are a
**JavaScript subset**: member access, literals, comparison with **strict** equality (`===`/`!==`),
boolean/`!` logic, and JS truthy/falsy — **no** function calls, assignment, or arbitrary code
(the engine is a sandboxed evaluator, never `eval()`). The scope exposes these namespaces:

| Namespace | Available where | Source |
| --- | --- | --- |
| `inputs.*` | everywhere | declared Flow `inputs` (CLI `--input`, calling `flow` task, defaults) |
| `env.*` | everywhere | resolved context `env` |
| `secrets.*` | only the names the task lists in `secrets` | resolved context `secrets` (opt-in per task) |
| `tasks.*` | everywhere | the Pipeline `state` — `tasks.NAME.field` reads a prior task's output |
| `item.*` (or the `as` alias) | inside a `map` inner task | the current element; `item.index` is the zero-based index |
| `case.*`, `output` | inside `eval` scorers/subject | the current dataset case; `output` is the subject's output |
| `matrix.*` | when run via `--matrix` sugar | the current matrix combination |

Resolution failures (unknown namespace key, type mismatch against a declared input) are
**ResolutionError**s (exit `4`); `lint` catches the statically-checkable ones (including
`produces`-typed `tasks.NAME.field` references) before a run.

### Secrets & masking

Masking is a **domain policy**, enforced at the port boundary so no adapter can leak a secret:

- The Masker holds a registry of **sensitive values** — every secret value resolved during a run.
- A task receives a secret **unmasked** (so it can use it) only if it lists the name in its
  `secrets` array; unrequested secrets are never resolved into that task's scope at all.
- **Every value leaving the core through an output port** — `EventSink` payloads, the final-state
  serialization to stdout, `RunStore` writes, log lines — is passed through the Masker, which
  redacts occurrences of any sensitive value (and within nested JSON). So even a task that *did*
  request a secret and echoes it cannot surface it in output.

Masking is defence-in-depth layered on the opt-in model: the schema decides *who gets* a secret;
the Masker guarantees it *never appears* in anything emitted.

### map, eval, hooks, produces (pure-core orchestration)

- **`map`** builds N child scopes (binding each element under `as`), runs the inner `task` through
  the runner under a **bounded [Scheduler](#concurrency-cancellation-timeouts)** (`concurrency`,
  default 1), and collects an **ordered** array (item order, not completion order). `continueOnError`
  records a failing item's error in its slot. The surrounding task list stays sequential.
- **`eval`** runs the `subject` once per `dataset` case (reusing the same bounded fan-out), applies
  each `scorer`, computes the per-case weighted mean and the aggregate `summary`, and gates on
  `threshold`. Scorers route by kind: `matcher` → pure MatcherEngine; `llmRubric` → `ChatModel`
  port; `exec`/`run` → `ProcessRunner` port. A missed `threshold` is a **RunFailure** (exit `1`) —
  the CLI's eval-as-gate behaviour.
- **MatcherEngine** implements the shared [Vitest matcher vocabulary](./SCHEMA.md) — the one
  primitive behind both `assert` (gate) and the `matcher` scorer (score 1.0/0.0). Pure, no I/O.
- **`produces`** is declarative; runtime conformance is **opt-in** (off by default, per the schema),
  while `lint` always uses it statically.

### State size limit

The whole Pipeline state is held **in memory** and threaded through tasks. To keep that bounded, the
serialized state has a **default cap of 512 MiB**; a merge that would exceed it **aborts the run**
(RunFailure, exit `1`) naming the offending task, rather than growing without limit. Raise the cap
with `--max-state-size`, the `limits.maxStateSize` config key, or `TMX_MAX_STATE_SIZE`.
`{ "blob": … }` outputs and large `map`/`eval` result arrays count toward the cap. v0 deliberately
has **no spill-to-disk / external `Blob` port** — an explicit, raisable limit is preferred over
silent external storage; revisit if real workloads need it.

## Driving side — use cases & adapters

The outside world enters through **application use cases** (the driving ports). Each is a thin
orchestration over the core + driven ports; the CLI is one adapter that maps commands to them.

| Use case (driving port) | Orchestrates | CLI adapter |
| --- | --- | --- |
| `RunFlow(ref, inputs, opts)` | load → resolve → (provision) → `PipelineRunner` → report → store | `tmx run` |
| `ValidateArtifacts(paths)` | load → `SchemaValidator` (kind-dispatch) | `tmx validate` |
| `LintFlow(ref)` | load → resolve → static interpolation/`produces`/secret checks | `tmx lint` |
| `InspectFlow(ref)` | load → resolve → render plan/inputs/secrets | `tmx inspect` |
| `ProvisionEnvironment(ref, method)` | select provider adapter → run method | `tmx env …` |
| `ManageProviders(op)` | registry read/write, manifest validation | `tmx provider …` |
| `QueryRuns(op)` | `RunStore` list/show/state/logs/prune | `tmx runs …` |
| `FormatArtifact(path, to)` | `SourceLoader` → re-emit in target format | `tmx fmt` |

Because these are ports, the **same core** runs unchanged behind a library import (embed it), an
HTTP server (host it), or another product's DSL ([`comparison.md` §9.5](./comparison.md#9-where-tmx-is-the-clear-choice)).

## Driven side — ports & adapters

The core depends on these secondary ports; v0 ships one built-in adapter each. The
**TaskDispatcher** is the seam that selects an executor port from a task's `type`:

| Task `type` | Domain op | Driven port | Built-in adapter |
| --- | --- | --- | --- |
| `exec` | run a command | `ProcessRunner` | OS process |
| `run` | run a script | `ProcessRunner` | OS process + language launchers |
| `fetch` | HTTP request | `HttpClient` | host HTTP client |
| `file` | filesystem op | `FileSystem` | local fs |
| `store` | object-store op | `ObjectStore` | S3-compatible SDK |
| `chat-completion` | LLM call | `ChatModel` | ChatCompletions client |
| `assert` | boolean gate | *(pure MatcherEngine)* | — none |
| `map` | bounded fan-out | *(core Scheduler + inner port)* | — none |
| `eval` | measure | MatcherEngine + `ChatModel` + `ProcessRunner` | mixed |
| `flow` | compose | *(recursion into `PipelineRunner`)* | — none |

Cross-cutting driven ports (not tied to a task type):

| Port | Responsibility | Built-in adapter(s) |
| --- | --- | --- |
| `SourceLoader` | parse a source file to the JSON model; `kind`-dispatch | YAML · JSON · JSONC · TOML |
| `ReferenceResolver` | resolve a `reference` string to a source | filesystem path (v0); registry *(deferred)* |
| `SchemaValidator` | validate artifacts / `produces` | JSON Schema 2020-12 |
| `SecretResolver` | resolve a `secretSource` | `env` · `file` · provider (`aws-sm`/`vault`/…) |
| `EnvironmentProvider` | `bootstrap`/`deploy`/`clean`/`destroy` | `BinaryProvider` · `FlowProvider` |
| `RunStore` | persist + query + prune runs | local `.tmx/runs` |
| `EventSink` | receive domain events | pretty (stderr) · ndjson (stdout) · final-state (stdout) |
| `Clock` | now / durations / timeouts | system clock |
| `IdGenerator` | run IDs | UUIDv7 |
| `Scheduler` | bounded concurrent execution | host concurrency (test: deterministic serial) |

Adding a backend (a GCS `ObjectStore`, a Vault `SecretResolver`, a sqlite `RunStore`) is a new
adapter behind an existing port — the core is untouched. The built-in **task set is fixed by the
schema enum**; user extension goes through `flow` import, not new task types (see
[Open questions](#open-questions) on a plugin-executor port).

## Environment & provider execution

The `environment` block is materialised by an `EnvironmentProvider` port with two adapters,
mirroring the [manifest's](./tmx-provider.schema.json) `type`:

- **`BinaryProvider`** — invokes the manifest's `binary` with the method's subcommand string,
  passing the resolved `environment` (and `options`) as input; the process's result is the method
  result. The CLI validates `environment.options` against the provider's `optionsSchema` first.
- **`FlowProvider`** — runs the method's inline tasks / referenced Flow **through the same
  `PipelineRunner`**. This is the recursion the schema implies: a provider method body inherits the
  entire task model (`map`, `eval`, `produces`) because it *is* a Flow. The port hides which adapter
  is in play.

**Ephemeral lifecycle** ([`CLI.md`](./CLI.md#how-run-relates-to-the-environment)). `RunFlow` wraps
the pipeline:

```
tmx run            → provider.deploy → PipelineRunner.run → provider.clean
tmx run --keep     → provider.deploy → PipelineRunner.run
tmx run --no-deploy→ PipelineRunner.run                         (reuse a standing env)
tmx run --local    → PipelineRunner.run                         (no provider; current process)
```

A failed provider method is an **EnvironmentError** (exit `5`), distinct from a pipeline
RunFailure (exit `1`). `clean`/`destroy` run on a best-effort basis even after a failed run, and the
context `destroy` hook still fires.

## Observability — events & reporters

The runner emits **one canonical event stream**; reporter adapters render it. Data goes to
**stdout**, progress to **stderr** ([WebCLI](./CLI.md#stdout--stderr-contract) stream separation).

| Event | When |
| --- | --- |
| `run.start` / `run.finish` | pipeline boundaries (`finish` carries status + total ms) |
| `task.start` / `task.finish` | around each task (`finish` carries status, ms, masked output) |
| `task.skip` | `if` evaluated falsy |
| `task.error` | a task failed (aborting, or recorded under `continueOnError`) |
| `map.item.finish` / `eval.case.finish` | per fan-out element / dataset case |
| `hook.start` / `hook.finish` | lifecycle hook execution |

Reporter adapters: **pretty** (human summary, stderr), **ndjson** (one event per line, stdout — for
CI/programmatic/LLM consumers), **final-state** (the merged JSON object, stdout). All pass through
the Masker. `--format` selects the stdout reporter; stderr progress is independent.

## Run store & retention

`RunStore` persists each run to `./.tmx/runs/<uuidv7>/` — a final-state snapshot plus the ndjson
event log. IDs are **UUIDv7** (time-ordered → chronological listings without a sort key). Records
are purged after a default **30 days**, applied opportunistically at the start of each `tmx run`
and on demand via `tmx runs prune`; configurable via `runs.retention` / `TMX_RUNS_RETENTION`
(`0`/`off` disables). It is a **record, not a journal** — no replay/durability. `--no-store` opts
out. (The local-fs adapter is one implementation; a sqlite or remote `RunStore` is a drop-in.)

## Concurrency, cancellation, timeouts

- **Bounded only.** The `Scheduler` enforces `map`/`eval` `concurrency` (and the global
  `--concurrency` cap); there is no unbounded or distributed parallelism. The test adapter runs
  serially for determinism.
- **Cancellation** propagates a cancel signal from the root: `--timeout` (via `Clock`) and SIGINT
  both trigger it. On cancel, the Scheduler stops dispatching new work, in-flight adapters are given
  a grace period then hard-stopped, the `destroy` hook fires, and the run exits `124` (timeout) or
  `130` (interrupt).
- **Per-task `timeout`** (`exec`/`run`/`fetch`/`store`) is enforced by the adapter under the same
  cancellation contract.

## Errors → exit codes

The core returns **typed error categories**; it knows nothing about exit codes. The CLI adapter is
the only place that maps them ([`CLI.md`](./CLI.md#exit-codes)):

| Core error category | Exit |
| --- | --- |
| `RunFailure` — task aborted, `assert` failed, `eval` threshold missed, state cap exceeded | `1` |
| *(CLI usage error)* | `2` |
| `ValidationError` — schema or `lint`, incl. a preflight task-validation failure | `3` |
| `ResolutionError` — ref/flow/provider not found, bad `${{ }}`/input | `4` |
| `EnvironmentError` — a provider method failed, or a preflight capability check failed | `5` |
| `Timeout` / `Interrupt` | `124` / `130` |

Keeping the mapping in the adapter is the hexagonal rule: an HTTP-server driving adapter would map
the *same* categories to status codes instead.

## Determinism & testability

The hexagonal payoff, made explicit:

- **Pure core, fake adapters.** The entire execution algorithm — sequencing, `if`, state merge,
  `map`/`eval`, masking, hooks — is unit-testable by injecting in-memory `ProcessRunner`,
  `HttpClient`, `ChatModel`, `Clock` and `IdGenerator`. No process, socket or disk required.
- **Reproducibility.** With `Clock`/`IdGenerator` injected and `--state-in` seeding, a run over
  deterministic adapters is reproducible end-to-end; only intrinsically nondeterministic adapters
  (LLM, network) vary, and those are exactly the swappable boundary.
- **Golden flows.** A conformance suite drives `RunFlow` with recorded adapters and asserts the
  event stream + final state — the basis for the "spec + conformance" asset in
  [`comparison.md` §9.5](./comparison.md#9-where-tmx-is-the-clear-choice).

## Embedding & sandboxing

Because the use cases are driving ports and every effect is a driven port:

- **Embed** the core as a library and supply your own adapters (your `ObjectStore`, your
  `SecretResolver`, your `ChatModel`).
- **Sandbox** a Flow by composing a restricted adapter set: a denying `ProcessRunner`, an
  allowlist `HttpClient`, a read-only `FileSystem`. The core cannot reach the host except through
  the ports you hand it — so an untrusted Flow is contained by construction, not by policy bolted on
  afterwards.

## Design decisions

1. **Pure core, effects at the edge.** The sequential JSON-state model has no I/O; processes,
   network, filesystem, clock and randomness arrive only via ports. _Chosen_ for testability,
   sandboxing, and the embeddable-DSL use case — the cost is the indirection of dependency
   injection at composition time.
2. **One executor port per side-effecting task type; `assert`/`map`/`flow` stay pure.** The
   built-ins *are* the adapters. _Chosen_ over a single generic "plugin" port (less type-safety) —
   the schema's fixed task enum makes the port set closed and known.
3. **`FlowProvider` recurses into the same runner.** A `flow`-type provider method is just a Flow,
   so it reuses `PipelineRunner` and inherits the whole task model. _Chosen_ over a separate
   provider mini-engine (would duplicate semantics and drift).
4. **Masking is enforced at the output boundary, not by callers.** The core registers sensitive
   values and every output port redacts. _Chosen_ over trusting each adapter to self-censor (one
   forgetful adapter leaks).
5. **Exit-code mapping lives in the driving adapter.** The core returns typed categories. _Chosen_
   so non-CLI hosts (HTTP, library) map the same categories their own way.
6. **Bounded concurrency only, via a `Scheduler` port.** Matches the schema's `map`/`eval` bound and
   keeps a deterministic serial test adapter. _Chosen_ over ambient host threading (non-deterministic
   tests, unbounded fan-out).
7. **`produces` runtime checking is opt-in.** Declarative by default (per the schema); `lint` uses
   it statically, `--check-produces=strict` enforces it at runtime. _Chosen_ to honour "no execution
   effect" while still offering a guardrail.
8. **The task enum stays closed (no plugin executor port for now).** User tasks go through `flow`
   import + `exec`/`run`, not a registered custom `type`. _Chosen_ to keep the port set known and the
   trust surface small; a custom-executor port is deferred, not designed out.
9. **Preflight validates and fails fast.** Every task is schema-validated, and the adapter-capability
   check runs, **before any side effect** — a malformed task is a `ValidationError` (exit `3`) and a
   missing real adapter (e.g. a working `ObjectStore` for a `store` task) is an `EnvironmentError`
   (exit `5`). _Chosen_ over discovering the problem mid-run; a half-run folder is the failure mode
   to avoid.
10. **Lifecycle hooks are one level deep.** A hook's tasks run through the same runner but do **not**
    recursively fire lifecycle hooks (a `change` hook mutating state does not re-trigger `change`).
    _Chosen_ to make hooks terminating and predictable (no hook-storm).
11. **In-memory state with a raisable cap; no spill port.** State is held in memory, capped at a
    documented default (512 MiB), raised via `--max-state-size` / config / env; exceeding it aborts
    (exit `1`). _Chosen_ over a spill-to-disk / external `Blob` port — an explicit, visible limit
    beats silent external storage. Revisit if real workloads demand it.

## Open questions

Runtime v0 review resolved the closed task enum, preflight validation + capability fail-fast,
one-level hooks, and the in-memory state cap — folded into [Design decisions](#design-decisions)
8–11. What remains open:

- **Concurrency & ordering of `map` side effects.** Output order is defined (item order); should the
  spec also constrain *observable* side-effect ordering under `concurrency > 1`, or leave it
  explicitly unspecified?
- **Host language.** **Resolved outside this draft:** the [`docs/specs/`](specs/00-overview.md)
  implementation spec commits to **Rust** (Tiger Style) and fixes the port idioms — traits, a typed
  `RunError`, async only at the adapter edge.

## Related

- [`CLI.md`](./CLI.md) — the command surface this runtime backs (every command maps to a use case)
- [`README.md`](../README.md) — the TMX model (Flow / Pipeline / Task / Context / Environment / Provider)
- [`SCHEMA.md`](./SCHEMA.md) — schema design decisions; the execution semantics here implement them
- [`tmx.schema.json`](./tmx.schema.json) · [`tmx-provider.schema.json`](./tmx-provider.schema.json) — the model the core executes
- [`comparison.md`](./comparison.md) — landscape positioning (the runtime realises the wedges in §9)
