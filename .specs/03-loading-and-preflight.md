# 03 — Loading and Preflight

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

Everything that happens **before the first side effect**: parsing a source into the JSON model,
dispatching it by `kind`, resolving references, assembling a directory into a Flow, validating every
artifact, and checking the host has the capabilities the Flow will use. Preflight either passes
wholesale or fails fast with nothing executed — a folder never runs half-way on a malformed task.

This realises [`RUNTIME.md` §Preflight](../RUNTIME.md#preflight--load--validate--capability-check) in
Rust. The ports used here (`SourceLoader`, `ReferenceResolver`, `SchemaValidator`) are detailed in
[06-ports-and-adapters.md](06-ports-and-adapters.md).

---

## Responsibilities

1. Parse a YAML / JSON / JSONC / TOML source into one `serde_json::Value` model.
2. Identify each artifact by its `kind` discriminator (or filename convention when `kind` is absent).
3. Resolve `environment` / `context` / `flow` / hook references to their source artifacts.
4. Assemble a **directory** target into a single Flow (shared `environment.*`/`context.*` + every
   other file as a task, in natural filename order).
5. Desugar the map form and `exec` shorthand into a fully-formed, ordered `Vec<Task>`.
6. Validate every artifact against the schema; any failure is a `ValidationError` (exit 3).
7. Compute the set of ports the Flow will touch and verify each bound adapter is **present and real**;
   a missing capability is an `EnvironmentError` (exit 5).

The output of a passing preflight is a `ResolvedFlow` ready for the
[runner](04-execution-engine.md), and a `CapabilitySet` the runner can trust.

---

## Source loading and `kind` dispatch

`SourceLoader` parses a file to the JSON model; one adapter per format, selected by extension. All
four produce the _same_ model, so everything downstream is format-agnostic (TMX's defining trait).

| `kind` (or convention)               | Schema (`$defs`)  | Rust target        |
| ------------------------------------ | ----------------- | ------------------ |
| `flow` (default for a top-level doc) | `flow`            | `Flow`             |
| `environment` (`environment.*`)      | `environment`     | `Environment`      |
| `context` (`context.*`)              | `context`         | `Context`          |
| `task` (any filename)                | `task`            | `Task`             |
| `provider`                           | provider manifest | `ProviderManifest` |

`kind` is **optional**; when absent, the loader falls back to filename convention (the reserved
`environment.*` / `context.*` names) and to "a top-level document with `tasks` is a Flow". Task files
may use **any** filename — identity is `kind`, or "not a reserved artifact". The map form preserves
source key order (the loader must feed keys to an order-preserving map; see
[01](01-domain-model.md#tasks)).

---

## Reference resolution

A `reference` is a string — a relative/absolute file path or a registered name. **v0 resolves
references as file paths only** ([`CLI.md`](../CLI.md#flow-resolution)); registered-name resolution is
out of scope in v0. The `ReferenceResolver` port:

- Resolves a path relative to the referring document's directory.
- Loads and `kind`-dispatches the target via `SourceLoader`.

Preflight's resolver chases the `environment` / `context` / hook references a Flow inlines; it does
not walk `flow`-type task imports. **Cyclic `flow` imports on the run path are caught by the
`FLOW_DEPTH_MAX` depth bound at execution time** — a cyclic import recurses until it trips the depth
cap, a typed error (see [04](04-execution-engine.md#bounded-flow-recursion)) — rather than by
chain-membership tracking in the resolver. Explicit chain-tracking cyclic-import detection is a
`lint`-time check (below), which reports a `cyclic_flow_import` diagnostic when a `use` reference
resolves back to a source already on the import chain.

The one v0 registry is the local provider map populated by `tmx provider add` (a name → path
mapping), not a remote namespace.

---

## Directory assembly

`tmx run <dir>` (and the folder-layout) assembles a directory into one sequential Flow:

1. A sibling `environment.*` / `context.*`, when present, becomes the shared environment/context
   (same-folder only — there is no inheritance from a parent folder).
2. Every other artifact is a **task**: `kind: task`, or (when `kind` is omitted) any file that is not
   a reserved `environment.*` / `context.*` / `flow.*`.
3. Tasks order by **natural filename order**: byte-wise ASCII comparison, except that maximal runs
   of ASCII digits compare as unsigned integers — so `task-2` precedes `task-10`, `build` precedes
   `deploy`. Case-sensitive, locale-independent, identical on every host.
4. Each task file is desugared and validated **before the run starts**.

A standalone task file (any name) is likewise validated, then wrapped into a one-task Flow.

---

## Validation

`SchemaValidator` checks every artifact against the data-model schema (Draft 2020-12), `kind`-
dispatched — the same dispatch as [`scripts/validate.sh`](../scripts/validate.sh), now in-process.
This backs `tmx validate` and runs inside preflight. A failure produces one or more `Diagnostic`s and
a `ValidationError`; in a directory run, **a single malformed task aborts the whole run before any
task executes** (the half-run folder is the failure mode preflight exists to prevent).

Limits are enforced here too, as validation rather than as runtime surprises: a Flow with more than
`TASKS_PER_FLOW_MAX` tasks, a literal `items`/`dataset` array longer than `FANOUT_WIDTH_MAX`, or a
document nested deeper than `JSON_DEPTH_MAX` is rejected at preflight with a diagnostic naming the
limit. (A collection produced by a `${{ }}` expression can only be checked once resolved at run
time, where the same violation is a `RunFailure` — see the
[limits table](04-execution-engine.md#limits).) A task `concurrency` above `CONCURRENCY_MAX` — or a
`--concurrency` flag above it — is likewise rejected here.

Structural expectations the schema cannot express are validated here as well: every array-form task
must carry a non-empty `name` (the map form's keys supply it) — a nameless task is a
`ValidationError` (`missing_task_name`) — and duplicate task names are a `ResolutionError` during
desugaring (see [01](01-domain-model.md#id-scheme)).

Unknown constructs are rejected **here, at validation**, not deferred to dispatch: a construct the
CLI cannot interpret — an unknown task `type`, field, or `with` shape — is a `ValidationError`
(exit 3) raised in preflight, before any task runs, rather than a surprise when the dispatcher
reaches it.

> **Not implemented — no spec-version gate.** There is no relaxed schema mode and no version bound:
> the build reports its supported spec version through `tmx version`, but a Flow is not matched
> against a spec version and no newer-spec compatibility warning is emitted at preflight.

### `lint` (static analysis beyond schema)

`lint` is a separate, deeper pass (still no side effects), backing `tmx lint`:

- Resolve `environment` / `context` / `flow` references and confirm they load.
- Walk every `${{ tasks.NAME.field }}` against the referenced task's `produces` schema, catching
  typos like `tasks.build.artifcat` — the static `produces` checking the schema docs promise.
- Flag inputs used-but-undeclared, and secrets used-but-not-listed in a task's `secrets`.
- Flag duplicate or missing task `name`s in the array form — the same checks preflight enforces,
  surfaced statically.
- Detect cyclic `flow` imports.
- Where a provider manifest has an `optionsSchema`, validate the environment's `options` against it.

`lint` emits `Diagnostic`s; `--strict` promotes warnings to errors. Both `validate` and `lint` exit
`3` on failure but differ in depth: `validate` is pure schema, `lint` is resolution + dataflow.

---

## Capability check

The final preflight step. The engine computes the set of ports the Flow will touch — from the task
`type`s used, recursing into `map`/`eval` inner tasks, `eval` scorer kinds (`llmRubric` →
`ChatModel`, `exec`/`run` → `ProcessRunner`), and lifecycle hook bodies (the context's hooks and the
environment's `bootstrap`) — and verifies each **bound adapter is present and real**, not a stub or
denying adapter. A declared `environment` provider requires the `EnvironmentProvider` port itself;
the check gates on the provider block's presence and does **not** recurse into the provider's own
method bodies.

```
CapabilitySet = { ports required by the flow }
for each required port:
    if the composed adapter is absent or a denying stub:
        return EnvironmentError(missing: <port>, for: <task type>)   # exit 5
```

A Flow that uses `store` requires a working `ObjectStore`; if one is not wired, preflight fails fast
naming the missing capability, rather than aborting mid-run at the first `store` task. This is what
makes sandboxing safe _and_ legible: a denying adapter set is reported up front, not discovered
half-way.

---

## Preflight flow

```
target (file | dir | name)
        │
        ▼
  SourceLoader.parse ──▶ JSON model ──▶ kind dispatch
        │
        ▼
  ReferenceResolver  (env/context/hook refs; v0 = file paths)
        │
        ▼
  directory assembly + desugar (map form, exec shorthand) ──▶ ordered Vec<Task>
        │
        ▼
  SchemaValidator (every artifact; limits) ──┬─ fail ─▶ ValidationError (exit 3)
        │ pass                                │
        ▼                                     │
  Capability check ──────────────────────────┴─ missing ─▶ EnvironmentError (exit 5)
        │ pass
        ▼
   ResolvedFlow + CapabilitySet  ──▶  PipelineRunner   (04)
```

---

## Implementation layout

`tmx-core/src/preflight.rs` orchestrates; it calls the `SourceLoader`, `ReferenceResolver`, and
`SchemaValidator` ports (adapters in `tmx-adapters/src/loader/`, `validate.rs`). Desugaring lives next
to the model in `tmx-schema`. The use cases `ValidateArtifacts`, `LintFlow`, and `InspectFlow`
(`tmx-core/src/usecases.rs`) are thin orchestrations over this pass.

---

## Assumptions and open questions

**Assumptions**

- Every supported source format has a loader that yields the identical JSON model; cross-format parity
  is a property the example corpus and CI already assert.
- A JSON-Schema-2020-12 validator crate exists that supports the schema features used (`$ref`,
  `allOf`/`if`/`then`, cross-file `$ref` for the provider manifest).

**Decisions**

- _`validate` and `lint` are split._ **`validate` is pure schema (`kind`-dispatch, a port of
  `scripts/validate.sh`); `lint` adds reference resolution and `produces`-based interpolation
  checking.** Chosen per [`CLI.md` decision 4](../CLI.md#design-decisions): two depths, both exit 3,
  distinct responsibilities.
- _Limits enforced at preflight, not mid-run._ **Over-limit task counts, fan-out widths, and JSON
  depths are `ValidationError`s before execution.** Chosen so a Flow that would blow a limit fails
  fast and legibly, consistent with Tiger Style "every limit explicit and checked".
- _Capability check before side effects._ **Missing real adapters are an `EnvironmentError` up
  front.** Chosen per [`RUNTIME.md` decision 9](../RUNTIME.md#design-decisions) over discovering the
  gap at the first failing task.
- _References are file paths in v0._ **No registry beyond the local provider map.** Chosen per
  [`CLI.md` decision 11](../CLI.md#design-decisions); a resolver spec is deferred.
- _Natural filename order is byte-wise + numeric-aware._ **Filenames compare byte-wise (ASCII,
  case-sensitive, locale-independent), with maximal runs of ASCII digits compared as unsigned
  integers.** Chosen so directory runs are reproducible across hosts — locale-aware collation
  would make task order host-dependent.
- _Unknown constructs fail fast at validation._ **An unknown construct (task `type`, field, or
  `with` shape) is a `ValidationError` (exit 3) at preflight — not a dispatch-time discovery.**
  Consistent with the other "fail fast in preflight, before side effects" decisions above. There is
  **no spec-version gate**: the supported spec version is reported by `tmx version`, but a Flow is
  not matched against a version bound and no newer-spec compatibility warning is emitted (a
  relaxed-mode compatibility warning is deferred, not implemented).

**Open questions**

- None currently.
