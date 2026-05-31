# TMX Schema — Review Draft

A first-pass JSON Schema for TMX **Flows**, **Contexts** and **Environments**, derived
from `README.md`. The goal is to give us something concrete to review and research
against — it is **not** frozen. Where the README was silent or ambiguous I made a
defensible choice and flagged it under [Open questions](#open-questions).

- Schema: [`tmx.schema.json`](./tmx.schema.json) (JSON Schema Draft 2020-12)
- Provider manifest schema: [`tmx-provider.schema.json`](./tmx-provider.schema.json)
- Examples: [`examples/`](./examples) — one combined Flow in **JSON / YAML / TOML / JSONC**
  (kept byte-for-byte equivalent), a mixed-format [`folder-layout/`](./examples/folder-layout),
  and a [`provider-manifest.yaml`](./examples/provider-manifest.yaml). All validated.
- Targets the **parsed JSON model**; the same schema applies whether the source was
  YAML, JSON, JSONC or TOML.

## Model at a glance

```
Flow (static)                              Pipeline (runtime — out of scope here)
├── environment?   inline | "ref"          the live state of a Flow as it executes
├── context?       inline | "ref"
└── tasks[]        required, runs in order
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
|                              | `task-N.*`                                                | `$defs/task`        |

Standalone files validate against the sub-definition of the same name — see
[`examples/standalone/`](./examples/standalone). (Tooling note: to validate a
standalone file, point a validator at `#/$defs/<name>` while keeping the schema's
`$defs` in scope, as the example validation does.)

## Tasks

Common envelope on every task: `kind?`, `name`, `description`, `type` (required),
`if`, `secrets`, `context` + `contextStrategy` + `contextPrecedence`, `output`,
`continueOnError`. Type-specific config lives under **`with`**, selected by `type`
via a discriminated union:

| `type`            | `with` shape         | Purpose                                                   |
| ----------------- | -------------------- | --------------------------------------------------------- |
| `exec`            | `execWith`           | Run a shell command                                       |
| `run`             | `runWith`            | Run a program/script in any language (`script` or `file`) |
| `fetch`           | `fetchWith`          | HTTP/HTTPS request                                        |
| `file`            | `fileWith`           | Read/write files                                          |
| `store`           | `storeWith`          | Read/write S3-compatible storage                          |
| `chat-completion` | `chatCompletionWith` | Call an LLM (ChatCompletions spec)                        |
| `assert`          | `assertWith`         | Assert values                                             |
| `flow`            | `flowWith`           | Import another Flow as a user-defined task                |

Control flow is intentionally minimal per the README: tasks run **in sequence**, and
the only branching is the per-task `if` skip (a JS-subset expression, truthy/falsy +
strict equality). No loops, no parallelism, no `needs`. Each task's output is merged
into the Pipeline state under its `name` (override with `output`).

**Secrets are opt-in per task.** All secrets are auto-masked in output everywhere; a
task receives a secret unmasked only if it lists the secret's name in its `secrets`
array. A task that names no secrets gets none in clear text.

## Context

`env`, `secrets`, and lifecycle `hooks` (`create`, `change`, `destroy`, `error`).
A hook body is an inline task list **or** a reference to a Flow that implements it.
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
`destroy`. Each method is a subcommand string (binary providers), a Flow reference, or
an inline list of TMX tasks (which `$ref` the task definition in `tmx.schema.json`). An
environment's `provider` field names the manifest to use. See
[`examples/provider-manifest.yaml`](./examples/provider-manifest.yaml).

## Design decisions (interpretations of the README)

1. **`with` wrapper for task config.** Type-specific fields are nested under `with`
   rather than placed at the task's top level. Keeps the discriminated union clean and
   avoids collisions with common fields. _Alternative:_ inline config.
2. **`type: "flow"` for user-defined tasks.** The README says user tasks "are
   implemented as Flows that can be imported." Modelled as a first-class task type with
   `use` (reference) + `input`.
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
   `kind` const on each `$defs` artifact; `kind` set on all examples.
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
  A: Yes, but no need to set until first pass spec is ready to go.

## Validating locally

```bash
python3 -m venv .venv && . .venv/bin/activate && pip install jsonschema check-jsonschema
check-jsonschema --schemafile docs/tmx.schema.json docs/examples/single-file-flow.json
```
