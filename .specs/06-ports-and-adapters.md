# 06 — Ports and Adapters

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

The driven side of the hexagon: every capability the core needs from the outside world is a **port**
(a trait owned by `tmx-core`), and each has one **built-in adapter** in `tmx-adapters`. The
`TaskDispatcher` is the seam that selects an executor port from a task's `type`. Adding a backend (a
GCS `ObjectStore`, a Vault `SecretResolver`) is a new adapter behind an existing port — the core is
untouched.

This realises [`RUNTIME.md` §Driven side](../RUNTIME.md#driven-side--ports--adapters). The driving
side (use cases → the CLI) is in [07-cli.md](07-cli.md).

---

## TaskDispatcher

`TaskDispatcher` maps a resolved task's `type` to the port that executes it. It is pure routing — no
I/O — and the place the closed task enum becomes a closed set of ports.

| Task `type` | Domain op | Driven port | Built-in adapter |
|---|---|---|---|
| `exec` | run a command | `ProcessRunner` | OS process |
| `run` | run a script | `ProcessRunner` | OS process + language launchers |
| `fetch` | HTTP request | `HttpClient` | host HTTP client (`reqwest`) |
| `file` | filesystem op | `FileSystem` | local fs |
| `store` | object-store op | `ObjectStore` | S3-compatible SDK |
| `chat-completion` | LLM call | `ChatModel` | ChatCompletions client |
| `assert` | boolean gate | *(pure `MatcherEngine`)* | — none |
| `map` | bounded fan-out | *(core + `Scheduler` + inner port)* | — none |
| `eval` | measure | `MatcherEngine` + `ChatModel` + `ProcessRunner` | mixed |
| `flow` | compose | *(recursion into `PipelineRunner`)* | — none |

`assert`/`map`/`flow` are pure-core and need no adapter. The dispatcher asserts exhaustiveness over the
enum, so a new task type cannot be added without wiring its port.

---

## Executor ports

Each port is an `async` trait method (the boundary is the only async layer). Signatures are sketches —
the precise types live in `tmx-core::ports::driven`. All ports return `Result<_, RunError>`; an
adapter never panics on a remote/host failure (that is a typed `RunError`), and bounds its captured
output by `CAPTURED_OUTPUT_MAX_BYTES`.

| Port | Method (sketch) | v0 adapter | Notes |
|---|---|---|---|
| `ProcessRunner` | `run(spec) -> ProcessOutput` | `OsProcessRunner` | `exec` (one command) and `run` (script in a language; default `bash`). Enforces per-task `timeout`. |
| `HttpClient` | `send(request) -> HttpResponse` | `ReqwestHttpClient` | `fetch`: method, headers, query, body (`bodyType`), `followRedirects`, bounded `retries`, `timeout`. |
| `FileSystem` | `op(FileOp) -> FileResult` | `LocalFileSystem` | `file`: read/write/append/delete/copy/move/exists; `encoding`. |
| `ObjectStore` | `op(StoreOp) -> StoreResult` | `S3ObjectStore` | `store`: get/put/delete/list/head; `endpoint`/`region`/`credentials`. |
| `ChatModel` | `complete(request) -> ChatResponse` | `ChatCompletionsModel` | `chat-completion` and the `llmRubric` scorer; ChatCompletions spec. |

`exec` vs `run`: `exec` runs a single shell command line; `run` runs a script/program in a named
language/interpreter, defaulting to `bash`, via inline `script` or a `file` path.

---

## Cross-cutting driven ports

Not tied to a single task type:

| Port | Responsibility | v0 adapter(s) |
|---|---|---|
| `SourceLoader` | parse a source file to the JSON model; `kind`-dispatch | YAML · JSON · JSONC · TOML |
| `ReferenceResolver` | resolve a `reference` string to a source | filesystem path (v0); registry *(out of scope, v0)* |
| `SchemaValidator` | validate artifacts / `produces` | JSON Schema 2020-12 |
| `SecretResolver` | resolve a `secretSource` | `env` · `file` · provider (`aws-sm`/`vault`/…) |
| `EnvironmentProvider` | `bootstrap`/`deploy`/`clean`/`destroy` | `BinaryProvider` · `FlowProvider` |
| `RunStore` | persist + query + prune runs | local `.tmx/runs` |
| `EventSink` | receive domain events | pretty (stderr) · ndjson (stdout) · final-state (stdout) |
| `Clock` | now / durations / timeouts | system clock |
| `IdGenerator` | run ids | UUIDv7 |
| `Scheduler` | bounded concurrent execution | tokio (test: deterministic serial) |

`RunStore`, `EventSink`, and masking are detailed in [08-errors-and-observability.md](08-errors-and-observability.md);
the `Scheduler` is in [05](05-fan-out-and-eval.md#the-scheduler); `SourceLoader`/`ReferenceResolver`/
`SchemaValidator` in [03](03-loading-and-preflight.md). `Clock` and `IdGenerator` are the determinism
seam: injected, they make a run reproducible (see [architecture-principles.md](architecture-principles.md#25-determinism-and-testability)).

---

## Secret resolution

`SecretResolver` resolves a `secretSource` to a value, which the Masker then registers as sensitive
(see [04](04-execution-engine.md#secrets--masking)). Sources: `env` (a host env var), `file` (a path),
or a named `provider` + `key` (`aws-sm`, `gcp-sm`, `vault`, …). The provider backend set is left
**open** per [`SCHEMA.md`](../SCHEMA.md#still-open); v0 ships `env` and `file` and a provider trait
seam. Secrets are **never** accepted as plain CLI flags (a process-table leak); ad-hoc injection sets a
host env var referenced by a `secretSource: { env: … }`.

---

## Environment and provider execution

The `environment` block is materialised by the `EnvironmentProvider` port, with two adapters mirroring
the [manifest's](../tmx-provider.schema.json) `type`:

- **`BinaryProvider`** — invokes the manifest's `binary` with the method's subcommand string, passing
  the resolved `environment` (and `options`) as input; the process result is the method result. The
  CLI validates `environment.options` against the provider's `optionsSchema` first.
- **`FlowProvider`** — runs the method's inline tasks / referenced Flow **through the same
  `PipelineRunner`**. A provider method body *is* a Flow, so it inherits the entire task model
  (`map`, `eval`, `produces`) and the [recursion depth bound](04-execution-engine.md#bounded-flow-recursion).
  The port hides which adapter is in play.

**Ephemeral lifecycle** ([`CLI.md`](../CLI.md#how-run-relates-to-the-environment)). `RunFlow` wraps the
pipeline:

```
tmx run            → provider.deploy → PipelineRunner.run → provider.clean
tmx run --keep     → provider.deploy → PipelineRunner.run
tmx run --no-deploy→ PipelineRunner.run                         (reuse a standing env)
tmx run --local    → PipelineRunner.run                         (no provider; current process)
```

A failed provider method is an **`EnvironmentError`** (exit 5), distinct from a pipeline `RunFailure`
(exit 1). `clean`/`destroy` run best-effort even after a failed run, and the context `destroy` hook
still fires.

The **provider lifecycle** (`bootstrap`/`deploy`/`clean`/`destroy`, driven by `tmx env …`) is the
*substrate* and is kept distinct from the **context lifecycle hooks** (`create`/`change`/`destroy`/
`error`), which are the *pipeline* and fire **inside** `tmx run`.

---

## Concurrency, cancellation, timeouts

- **Bounded only.** The `Scheduler` enforces `map`/`eval` `concurrency` and the global `--concurrency`
  cap; there is no unbounded or distributed parallelism. The test adapter runs serially.
- **Cancellation** propagates a cancel signal from the root: `--timeout` (via `Clock`) and SIGINT both
  trigger it. On cancel, the Scheduler stops dispatching new work, in-flight adapters get a grace
  period (`CANCEL_GRACE_MS`, default 5 000 ms; `--grace <dur>` overrides) then a hard stop, the
  `destroy` hook fires, and the run exits `124` (timeout) or `130` (interrupt). In Rust this is a
  cancellation token threaded into every adapter call and awaited alongside the work.
- **Per-task `timeout`** (`exec`/`run`/`fetch`/`store`) is enforced by the adapter under the same
  cancellation contract.

---

## Adding a backend

A new backend is a new adapter implementing an existing port — a GCS `ObjectStore`, a Vault
`SecretResolver`, a sqlite `RunStore`. The core is untouched: the composition root names the new
concrete type, nothing else changes. The built-in **task set is fixed by the schema enum**; user
extension goes through `flow` import, not new task types (no plugin-executor port in v0).

---

## Assumptions and open questions

**Assumptions**

- One adapter per port is sufficient for v0; multiple adapters behind one port (e.g. provider-specific
  `ObjectStore`s) are selected at composition, not at runtime by the core.
- `reqwest` + the S3 SDK + tokio are acceptable dependencies for the default build; sandboxed/minimal
  builds drop them via features.

**Decisions**

- *One executor port per side-effecting task type; `assert`/`map`/`flow` stay pure.* **The built-ins
  *are* the adapters.** Chosen per [`RUNTIME.md` decision 2](../RUNTIME.md#design-decisions) over a
  single generic plugin port: the closed task enum makes the port set closed and known.
- *`FlowProvider` recurses into the same runner.* **A `flow` provider method is just a Flow.** Chosen
  per [`RUNTIME.md` decision 3](../RUNTIME.md#design-decisions) over a separate provider mini-engine
  that would drift from the runner's semantics.
- *Adapters return typed errors, never panic on host failure.* **A remote/host failure is a
  `RunError`; a panic is reserved for a broken invariant.** Chosen so the error→exit mapping stays
  meaningful and the process never aborts on an expected failure.
- *Secret provider backends stay open.* **A trait seam with `env`/`file` built in; `aws-sm`/`vault`
  are additional adapters.** Chosen per [`SCHEMA.md`](../SCHEMA.md#still-open): enumerate later, keep
  the seam now.
- *Cancellation grace period defaults to 5 s.* **In-flight adapters get `CANCEL_GRACE_MS` (default
  5 000 ms) between the cancel signal and the hard stop, overridable via `--grace <dur>`.** Chosen
  as long enough for a clean HTTP/process shutdown without holding a cancelled run hostage.
- *Plugin executors, if ever added, are external processes — never in-process code.* The feature
  itself stays deferred ([`RUNTIME.md` decision 8](../RUNTIME.md#design-decisions)), but its trust
  boundary is fixed now so a future design cannot drift toward in-process loading. **A plugin task
  is exactly as trusted as an `exec` task: an external process with declared inputs, bounded
  output, masked emission, and no reach into the engine.** Concretely, a plugin would be a single
  `PluginExecutor` driven port where:
  - the plugin is a separate **binary invoked per task** (no dylib/FFI, no in-process interpreter),
    mirroring the `BinaryProvider` model — plugin code never executes in the engine's address
    space, so `#![forbid(unsafe_code)]` and the closed core stay meaningful;
  - it registers via a **manifest** (a sibling of [`tmx-provider.schema.json`](../tmx-provider.schema.json)):
    the `type` it provides, its binary, an `optionsSchema` for its `with` block (validated at
    preflight, like provider options), and an optional `produces` for its output;
  - it receives **only the resolved, interpolated `with` plus the secrets the task listed**, as
    JSON on stdin — never the Pipeline state, other tasks' outputs, or unrequested secrets; its
    stdout is treated like any adapter result (bounded by `CAPTURED_OUTPUT_MAX_BYTES`, normalised,
    Masker-redacted, subject to the state cap and per-task `timeout`);
  - execution routes through the existing **`ProcessRunner` port**, so a sandboxed composition
    that injects a denying `ProcessRunner` automatically denies all plugins — no new sandbox
    surface;
  - the **capability check** requires the plugin's manifest to be registered and its binary
    present, failing preflight with an `EnvironmentError` naming the missing plugin.

**Open questions**

- None currently.
