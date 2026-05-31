# TMX Schema — Review Draft

A first-pass JSON Schema for TMX **Flows**, **Contexts** and **Environments**, derived
from `README.md`. The goal is to give us something concrete to review and research
against — it is **not** frozen. Where the README was silent or ambiguous I made a
defensible choice and flagged it under [Open questions](#open-questions).

- Schema: [`tmx.schema.json`](./tmx.schema.json) (JSON Schema Draft 2020-12)
- Provider manifest schema: [`tmx-provider.schema.json`](./tmx-provider.schema.json)
- Examples: [`examples/`](./examples) — one combined Flow in **JSON / YAML / TOML / JSONC**
  (kept semantically identical), a mixed-format [`folder-layout/`](./examples/folder-layout),
  a [`provider-manifest.yaml`](./examples/provider-manifest.yaml), and name-keyed-map /
  `exec`-shorthand task examples ([`map-tasks.yaml`](./examples/map-tasks.yaml),
  [`shorthand-tasks.json`](./examples/shorthand-tasks.json)). All validated.
- Targets the **parsed JSON model**; the same schema applies whether the source was
  YAML, JSON, JSONC or TOML.

## Model at a glance

```
Flow (static)                              Pipeline (runtime — out of scope here)
├── environment?   inline | "ref"          the live state of a Flow as it executes
├── context?       inline | "ref"
└── tasks          required — ordered array or name-keyed map
```

A **Flow** is the only top-level document. `tasks` is the only required field;
`environment` and `context` are optional and may be either inlined or given as a
string **reference** to a standalone file/name (so they can be reused).

### Artifact → schema mapping

The README describes two authoring styles. Both map onto the same `$defs`:

| Authoring style              | File(s)                                                   | Validates against   |
| ---------------------------- | --------------------------------------------------------- | ------------------- |
| Single combined file         | `flow.yaml` with `environment`/`context`/`tasks` sections | root (`$defs/flow`) |
| Standalone files in a folder | `environment.*`                                           | `$defs/environment` |
|                              | `context.*`                                               | `$defs/context`     |
|                              | _any filename_ (e.g. `task-N.*`)                          | `$defs/task`        |

Standalone files validate against the sub-definition of the same name — see
[`examples/standalone/`](./examples/standalone). (Tooling note: to validate a
standalone file, point a validator at `#/$defs/<name>` while keeping the schema's
`$defs` in scope, as the example validation does.)

**Task filenames are free.** The `task-N.*` naming above (and in the examples) is
illustrative only — a task file may use **any** name. Its `kind` (or, when `kind` is
omitted, the fact that it is run/loaded as a task) identifies it, and it is validated
against `$defs/task` **before it runs**. The reserved names `environment.*` / `context.*`
remain conventional for the shared folder artifacts.

## Tasks

Common envelope on every task: `kind?`, `name`, `description`, `type` (required),
`if`, `secrets`, `context` + `contextStrategy` + `contextPrecedence`, `output`,
`produces?`, `continueOnError`. Type-specific config lives under **`with`**, selected
by `type` via a discriminated union:

| `type`            | `with` shape         | Purpose                                                   |
| ----------------- | -------------------- | --------------------------------------------------------- |
| `exec`            | `execWith`           | Run a shell command                                       |
| `run`             | `runWith`            | Run a program/script in any language (`script` or `file`) |
| `fetch`           | `fetchWith`          | HTTP/HTTPS request                                        |
| `file`            | `fileWith`           | Read/write files                                          |
| `store`           | `storeWith`          | Read/write S3-compatible storage                          |
| `chat-completion` | `chatCompletionWith` | Call an LLM (ChatCompletions spec)                        |
| `assert`          | `assertWith`         | Assert values (boolean gate)                              |
| `map`             | `mapWith`            | Bounded fan-out of an inner task over a collection        |
| `eval`            | `evalWith`           | Score a subject over a dataset (measurement + scorecard)  |
| `flow`            | `flowWith`           | Import another Flow as a user-defined task                |

Control flow is intentionally minimal per the README: tasks run **in sequence**, the
only branching is the per-task `if` skip (a JS-subset expression, truthy/falsy +
strict equality), and there is no `needs`/DAG. The **one deliberate exception** is the
`map` task, which performs *bounded* iteration — running its inner task once per element
of a collection (optionally with bounded `concurrency`) and collecting the results — but
this does not reorder the surrounding sequential task list, and there is still no general
branching, DAG, or unbounded parallelism. Each task's output is merged into the Pipeline
state under its `name` (override with `output`); a task may declare a `produces` JSON
Schema describing that output so downstream `${{ tasks.NAME.field }}` references can be
statically linted.

**Secrets are opt-in per task.** All secrets are auto-masked in output everywhere; a
task receives a secret unmasked only if it lists the secret's name in its `secrets`
array. A task that names no secrets gets none in clear text.

## Context

`env`, `secrets`, and lifecycle `hooks` (`create`, `change`, `destroy`, `error`).
A hook body is an inline set of tasks (an ordered array **or** a name-keyed map), a
reference to a Flow, or a `{ use, inputs }` Flow import.
Secrets are auto-masked; tasks must declare which ones they need (see Tasks). On
inheritance, `env`/`secrets`/`hooks` merge **independently** at the key level;
`contextPrecedence` decides who wins a collision (`local` by default).

## Environment

Declarative substrate for provisioning a Pipeline: `os`, `arch`, `platform`,
`provider`, `runtime`, `image`, `resources`, and `bootstrap` tasks. Because
"environment-specific options will be unique to each provider/platform," the
environment object is **open** (`additionalProperties: true`) and carries a dedicated
free-form `options` block for provider extensions. The provider _methods_
(`bootstrap`/`deploy`/`clean`/`destroy`) belong to the provider implementation and are
modelled separately — see below.

## Environment provider manifest

The provider contract is its own artifact: [`tmx-provider.schema.json`](./tmx-provider.schema.json).
A manifest declares `name`, `type` (`binary` | `flow`), an optional `binary` path, an
optional `optionsSchema` (so the CLI can validate `environment.options` against the
chosen provider), and the four required `methods`: `bootstrap`, `deploy`, `clean`,
`destroy`. Each method is a subcommand string (binary providers), a Flow reference, a
`{ use, inputs }` Flow import, or an inline set of TMX tasks — an ordered array or a
name-keyed map, with the same `exec` string shorthand, all `$ref`-ing the task definition
in `tmx.schema.json`. Because they `$ref` that one definition, provider method bodies inherit
the **full task model automatically** — including the `map` and `eval` task types and the
`produces` contract added in 0.2.0 (verified: the example's `bootstrap` uses `map`, its `deploy`
uses `produces`, and an `eval` method body validates). An environment's `provider` field names
the manifest to use. See [`examples/provider-manifest.yaml`](./examples/provider-manifest.yaml).

## Design decisions (interpretations of the README)

1. **`with` wrapper for task config.** Type-specific fields are nested under `with`
   rather than placed at the task's top level. Keeps the discriminated union clean and
   avoids collisions with common fields. _Alternative:_ inline config.
2. **`type: "flow"` for user-defined tasks.** The README says user tasks "are
   implemented as Flows that can be imported." Modelled as a first-class task type with
   `use` (reference) + `inputs`.
3. **References are plain strings.** `environment`, `context`, hook bodies and flow
   imports accept a string path/name. No registry/URI scheme is assumed yet.
4. **Strict where shapes are known, open where they aren't.** Most objects are
   `additionalProperties: false` to catch typos during review; `environment`,
   `options`, `secretSource`, store `credentials`, and `chat-completion` are open
   because they front provider-specific or fast-moving APIs.
5. **`${{ ... }}` interpolation is assumed but not validated.** The schema treats
   interpolated values as strings; expression syntax/semantics are left to the engine.
6. **Pipeline is out of scope.** This schema covers the _static_ Flow definition only.
   The runtime Pipeline state object (and the JSON in/out contract — `message`/`blob`
   wrapping of non-JSON output) is documented in the schema descriptions but not
   itself schematised.

## Resolved decisions

These were open questions in the first draft; now answered and reflected in the schema.

1. **Standalone files are self-identifying.** Every artifact accepts an optional `kind`
   (`flow` | `environment` | `context` | `task`, and `provider` for manifests) so one
   validator can dispatch by `kind` instead of relying on filename. _Reflected:_ a
   `kind` const on each `$defs` artifact; `kind` set on the examples (it is optional, so
   `minimal-flow.json` omits it to demonstrate that).
2. **Context merges independently, local-wins by default.** `env`/`secrets`/`hooks`
   merge as independent sections at the key level. On a collision the in-file/`local`
   value wins by default; set `contextPrecedence: inherited` to let the parent/folder
   value override. _Reflected:_ `contextStrategy: merge|replace` + new
   `contextPrecedence: local|inherited` on the task envelope.
3. **`if` is a JS-subset expression.** Truthy/falsy semantics with strict equality
   (`===`). _Reflected:_ `if` description + examples updated to `!==`/strict form. (The
   grammar is engine-enforced, not schema-validated.)
4. **Output is merged by task `name`.** `state[name] = output`; the optional `output`
   field overrides the key. _Reflected:_ `name`/`output` descriptions.
5. **`exec` vs `run`.** `exec` = a single shell command; `run` = a script in a
   named language/interpreter. `run.language` now defaults to **`bash`**. _Reflected:_
   `runWith.language` default + descriptions.
6. **Provider contract is a separate schema.** `bootstrap`/`deploy`/`clean`/`destroy`
   live in [`tmx-provider.schema.json`](./tmx-provider.schema.json) with an optional
   `optionsSchema`. _Reflected:_ new schema + example.
7. **Secrets auto-masked, opt-in per task.** Secrets are masked in all output; a task
   declares the secret names it needs unmasked via its `secrets` array. _Reflected:_
   task `secrets` array + context/secret descriptions.
8. **Flows declare input variables.** A Flow may declare `inputs` (name → spec with
   optional `type`/`description`/`required`/`default`), supplied at invocation from the
   CLI (`--input key=value`, repeatable) or by a calling `flow` task. The value passed at
   a call site is standardised on an `inputs` object (renamed from the earlier `input`)
   across `flowWith`, lifecycle `hook` `{use, …}` bodies, and the provider manifest's
   `method` `{use, …}` body. Inputs are read inside the Flow via `${{ inputs.NAME }}`.
   _Reflected:_ `flow.inputs` + new `inputSpec` def; `input` → `inputs` at every flow
   call site (core + provider schemas) and in the examples.
9. **Task collections are array-or-map.** A set of tasks may be expressed **either** as an
   ordered array **or** as a name-keyed map (object) where the **key is the task name** (the
   task object then need not repeat `name`). The array form is explicitly ordered; the map
   form runs in the **source document's key order**. This applies everywhere a list of tasks
   is accepted: a Flow's `tasks`, lifecycle hook bodies, environment `bootstrap`, and the
   provider manifest's inline-task `method`. _Reflected:_ `taskList` kept as the array form;
   new `tasks` def is `oneOf: [taskList, <name-keyed map>]` used by `flow.tasks` and `hook`;
   provider `method` mirrors the map branch. See [`examples/map-tasks.yaml`](./examples/map-tasks.yaml).
10. **`exec` string shorthand (map form only).** In the **map form** of a task collection, a
   task value may be a plain **string** instead of a task object; it is shorthand for an
   `exec` task that runs the string as a shell command, with the map key as the task name
   (`{ "build": "npm run build" }` ≡ `{ "build": { "type": "exec", "with": { "command": "npm run build" } } }`).
   The shorthand is **map-only** — array (`taskList`) items must always be full task objects.
   _Reflected:_ the map branch's `additionalProperties` in `tasks` is now `oneOf: [ {$ref: task}, {type: string} ]`;
   the provider manifest's inline-task-map `method` branch mirrors it. See
   [`examples/shorthand-tasks.json`](./examples/shorthand-tasks.json) and the mixed map in
   [`examples/map-tasks.yaml`](./examples/map-tasks.yaml).
11. **`assert` uses Vitest `expect` matchers.** Each assertion is
   `{ actual, matcher, expected?, not?, message? }`, where `matcher` is a
   [Vitest matcher](https://vitest.dev/api/expect.html) (`toBe`, `toEqual`, `toContain`,
   `toHaveProperty`, …) and `not: true` mirrors the `.not` modifier. Mock- and
   promise-only matchers (`toHaveBeenCalled`, `resolves`, `rejects`, …) are excluded as
   they don't apply to asserting plain values. _Reflected:_ `assertion.matcher` enum +
   `not`, replacing the earlier ad-hoc operator set.
12. **`chat-completion` endpoint is `apiUrl` only.** The endpoint is given as a single full
   URL (`apiUrl`, e.g. `https://api.openai.com/v1/chat/completions`); there is no separate
   `baseUrl`. _Reflected:_ `chatCompletionWith.apiUrl` (the object stays open via
   `additionalProperties: true` for provider-specific params).

## Competitiveness-pass additions

Three task-model additions made during spec authoring to close gaps against shipping tools
(local runners, CI matrices, Step Functions, and LLM-eval harnesses) while preserving the
declarative, sequential-by-default identity. The rationale lives here; the shapes are in the
schema and worked in [`examples/map-fanout.yaml`](./examples/map-fanout.yaml),
[`examples/eval.yaml`](./examples/eval.yaml), and [`examples/typed-output.yaml`](./examples/typed-output.yaml).

13. **`map` — bounded fan-out (the one concession to non-linear execution).** Testing and
   evaluations are inherently "run this over N items," which strict sequential-only could not
   express — an internal contradiction with the README's stated use cases. `map` runs a single
   inner task (or `flow` import) once per element of `items`, binding each element under `as`
   (default `item`), with optional bounded `concurrency` and `continueOnError`, collecting an
   ordered array under the task name. Deliberately *bounded*: it does not introduce general
   branching, a DAG/`needs`, or unbounded parallelism, and it does not reorder the surrounding
   list. _Reflected:_ `map` in the `type` enum + `allOf` branch; new `mapWith` def; softened
   "no parallel execution" wording in `taskList`/`tasks`.

14. **`produces` — optional typed output contract.** TMX's headline is structured JSON
   dataflow, but task outputs were untyped (like Step Functions / n8n). A task may now declare a
   `produces` JSON Schema for its output. Purely declarative (no execution effect); it enables
   static linting of `${{ tasks.NAME.field }}` references before a run, editor autocomplete, and
   an optional runtime conformance check. This moves TMX toward typed-dataflow tools (Flyte,
   Dagster) without requiring a programming language. _Reflected:_ `produces` on the common task
   envelope (an embedded Draft 2020-12 schema object).

15. **`eval` is its own task type, distinct from `assert`.** Considered folding model-graded
   scoring into `assert` as extra matchers, but the two are different verbs with different
   output contracts and failure semantics: **`assert` GATES** (boolean, aborts on failure on a
   single known value); **`eval` MEASURES** (continuous `0..1` scores + aggregate metrics over a
   dataset, emitting a scorecard, failing only against an explicit `threshold` policy). Conflating
   them would distort assert's clean boolean identity and muddy its abort-on-fail semantics, and a
   Vitest matcher (deterministic predicate) is a category apart from an LLM rubric (probabilistic
   score). The recognised industry shape is `Eval(dataset, subject, scorers) → scorecard`
   (promptfoo, Braintrust, OpenAI Evals). To avoid a parallel vocabulary, **matchers are the
   shared primitive**: the matcher enum was extracted to a `matcherName` def reused by both
   `assertion.matcher` and the `eval` `matcher` scorer. `eval` reuses `map`'s bounded fan-out for
   its dataset (`concurrency`). _Reflected:_ `eval` in the `type` enum + `allOf` branch; new
   `evalWith`, `scorer`, `evalThreshold` defs; `matcherName` extracted and referenced from both
   `assertion` and `scorer`.

### Interpretation notes (flag if you'd prefer otherwise)

- `kind` is **optional**, not required — so minimal files (`{ "tasks": [...] }`) still
  validate. Say the word and I'll make it required on standalone files.
- `contextPrecedence` is a single flag for the whole context. If you want per-section
  precedence (e.g. `env` local-wins but `secrets` inherited-wins), that needs a richer
  object — not modelled yet.
- Provider `method` bodies `$ref` the canonical task definition across files, so
  validating a manifest requires both schemas in the resolver (the example validation
  wires this up).

## Still open

- Does `change` fire on _every_ state mutation (per task) or only on externally visible
  changes? Affects hook-storm potential.
  A: Fires at the end of each task, _if_ the state changes (ie if a task is skipped it won't fire)
- Secret `provider` backends (`aws-sm`, `vault`, …) — enumerate a supported set, or
  keep open?
  A: Keep open for now.
- Versioning: should `version`/schema `$id` carry a spec version for forward-compat?
  A: **Resolved — yes, in the `$id` path.** Both schemas now carry the spec version as a
  path segment (`https://tmx.dev/schemas/0.2.0/tmx.schema.json`), so distinct versions are
  distinct resources that can coexist (forward-compat), and the chosen `$id` is recorded in a
  top-level `$comment`. The provider schema's cross-`$ref`s to the task definition are pinned to
  the same versioned `$id`. Current version: **0.2.0 (draft)** — bumped from the implicit 0.1.0
  by the `map`/`eval`/`produces` additions. A Flow document does not declare its target version;
  the loader selects the schema. (Alternative considered: keep a stable `$id` and add a separate
  `version` field — rejected for weaker forward-compat, but it would avoid the provider-ref churn
  on each bump.)

## Validating locally

The canonical validator is [`scripts/validate.sh`](../../scripts/validate.sh) — it checks
both schemas, every example (dispatched by `kind`), and cross-format parity. To validate a
single file against the schema with `check-jsonschema` instead:

```bash
python3 -m venv .venv-tmx && . .venv-tmx/bin/activate && pip install jsonschema check-jsonschema
check-jsonschema --schemafile docs/tmx.schema.json docs/examples/single-file-flow.json
```
