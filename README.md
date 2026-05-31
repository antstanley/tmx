# TMX — A modern task runner

A flexible task runner that can run anywhere, for any use case where a set of
predefined steps need to be executed. Potential use cases include:

- CI/CD
- Testing
- Evaluations
- Configuration
- Deployment

It can be a replacement for Makefiles, Bash scripts, GitHub Actions Workflows, and other
task runners. It can also work in conjunction with them.

TMX defines **Flows** and **Pipelines**. A Flow is a static definition of tasks, context
and environment. A Pipeline is the state of a Flow at runtime.

> **Status: early design phase.** This repository currently contains the draft data model
> as JSON Schema plus worked examples — no runtime yet. The concrete shapes and decisions
> below are captured in [`docs/tmx.schema.json`](./docs/tmx.schema.json) and
> [`docs/tmx-provider.schema.json`](./docs/tmx-provider.schema.json); the rationale and
> still-open questions live in [`docs/SCHEMA.md`](./docs/SCHEMA.md).

## Concepts

| Term | Meaning |
| --- | --- |
| **Flow** | Static definition: `environment?` → `context?` → `tasks` (only `tasks` is required). |
| **Pipeline** | The live state of a Flow as it runs — a JSON object that each task reads and updates. |
| **Task** | One step. Consumes the Pipeline state, returns JSON that is merged back in. |
| **Context** | Env vars, secrets and lifecycle hooks shared by a Flow's tasks. |
| **Environment** | Declarative description of *where* a Flow runs; materialised by a provider. |
| **Provider** | Implements an environment (binary or Flow) via standard lifecycle methods. |

## Source formats

A Flow and its artifacts can be authored in **YAML, JSON, JSONC or TOML**. All four parse
to the same JSON model, so the schema applies regardless of source format. The
[`docs/examples`](./docs/examples) directory contains the same Flow in all four formats,
kept semantically identical.

Every artifact may carry an optional **`kind`** discriminator
(`flow` | `environment` | `context` | `task`, and `provider` for manifests). When present,
a single validator/loader can dispatch a file to the right schema instead of relying on
filename convention. It is optional, so a minimal `{ tasks: [...] }` file (array form) — or
its name-keyed map equivalent `{ tasks: { build: {...} } }` — is still valid.

## Flows

Flows are defined as a hierarchy of Environment → Context → Tasks. Only the tasks are
required. Environment, Context and Tasks can be defined within a single file as separate
sections, **or** as standalone files in a folder used as default configs with inheritance:

```
<flow_folder>
   |
   |- environment.[yaml|json|jsonc|toml]   <- where tasks run. Not required.
   |- context.[yaml|json|jsonc|toml]       <- shared env vars, secrets, lifecycle hooks.
   |- task-1.[yaml|json|jsonc|toml]        <- any filename; inherits folder env + context.
   |- build.[yaml|json|jsonc|toml]         <- task filenames are free (task-N is illustrative).
   |- deploy ...
```

In this layout all tasks inherit the environment and context from the standalone files.
**Task filenames are arbitrary** — the `task-N` naming is just an example; a task file is
identified by its `kind` (or by being run as a task) and is validated against the task
schema before it runs. The reserved names `environment.*` / `context.*` stay conventional
for the shared folder artifacts. Running the folder (`tmx run <folder>`) runs **all** its
tasks in filename order, sharing the folder-level environment and context. Where an inline
artifact and a `reference` are both allowed, a **reference** is just a string — a
relative/absolute file path or a registered name (e.g. `./context.yaml` or
`my-org/base-context`).

Inheritance is scoped to the **same folder** only; there is no inheritance from a parent
or root folder.

The smallest useful Flow:

```yaml
# hello.yaml
tasks:
  - name: greet
    type: exec
    with:
      command: echo "hello from TMX"
```

### Tasks

Tasks are the steps a Flow executes. TMX defines a number of built-in tasks and lets
users define their own.

Every task consumes the Pipeline state as a JSON object and returns JSON output. If the
output is not valid JSON, the runner wraps it: plain text becomes `{ "message": "..." }`
and binary becomes `{ "blob": <bytes> }`. Each task's output is merged into the Pipeline
state **under the task's `name`** (`state[name] = output`; override the key with
`output`), making it available to subsequent tasks.

Tasks run **in sequence**, one after another. There is no branching or DAG (`needs`); control
flow is deliberately minimal — a per-task `if` skip, plus the **bounded iteration** of the
[`map`](#bounded-fan-out-map) task, which fans a single inner task out over a collection
(optionally with bounded concurrency) without changing the order of the surrounding list.

A Flow's `tasks` may be given in **either** of two forms:

- an **ordered array** of task objects (explicitly ordered — tasks run top to bottom), or
- a **name-keyed map** (object) where each **key is the task's name** and its value is the
  task object (which then need not repeat `name`).

Both forms are equivalent in what they can express. The difference is ordering: the array
form is explicitly ordered, while the map form runs in the **source document's key order**
(the order keys appear in the YAML/JSON/JSONC/TOML file). The same array-or-map choice
applies anywhere a set of tasks is accepted — lifecycle hooks, environment `bootstrap`,
and provider manifest methods.

```yaml
# array form — explicitly ordered
tasks:
  - name: build
    type: exec
    with:
      command: npm run build

# map form — key is the task name, runs in key order
tasks:
  build:
    type: exec
    with:
      command: npm run build
```

**`exec` string shorthand (map form only).** In the **map form**, a task value may be a
plain **string** instead of a task object. The string is shorthand for an `exec` task that
runs it as a shell command, with the map key as the task name. So:

```json
{ "tasks": { "build": "npm run build" } }
```

is equivalent to the full `exec` task object:

```json
{ "tasks": { "build": { "type": "exec", "with": { "command": "npm run build" } } } }
```

The shorthand is **map-only** — array items must always be full task objects.

**Common task fields** (the envelope shared by every task):

| Field | Description |
| --- | --- |
| `type` | **Required.** One of the built-in types below, or `flow`. |
| `name` | Identifier; also the Pipeline-state key its output is merged under. |
| `description` | Optional human description. |
| `if` | Skip condition — a JavaScript-subset expression (truthy/falsy, strict `===`) evaluated against the Pipeline state. |
| `secrets` | Names of context secrets this task needs **unmasked** (see [Context](#context)). |
| `context` | Task-level context overrides (inline or reference). |
| `contextStrategy` | `merge` (default) or `replace` — how task context combines with inherited context. |
| `contextPrecedence` | `local` (default) or `inherited` — who wins a key collision during `merge`. |
| `output` | Override the Pipeline-state key for this task's output (defaults to `name`). |
| `produces` | Optional JSON Schema for this task's output — enables static linting of downstream `${{ tasks.NAME.field }}` references, autocomplete, and an optional runtime conformance check (see [Typed task output](#typed-task-output-produces)). |
| `continueOnError` | `false` (default) aborts the Pipeline on failure; `true` records the error and continues. |
| `with` | Type-specific configuration (shape determined by `type`). |

**Built-in task types** and their `with` configuration:

| `type` | Purpose | Key `with` fields |
| --- | --- | --- |
| `exec` | Run a single shell command | `command`*, `args`, `shell`, `cwd`, `env`, `timeout` |
| `run` | Run a script in a named language | `language` (default `bash`), `script` \| `file`*, `args`, `env`, `cwd`, `timeout` |
| `fetch` | HTTP/HTTPS request | `url`*, `method`, `headers`, `query`, `body`, `bodyType`, `timeout`, `followRedirects`, `retries` |
| `file` | Read/write files | `operation`* (read/write/append/delete/copy/move/exists), `path`*, `content`, `encoding`, `destination` |
| `store` | S3-compatible object storage | `operation`* (get/put/delete/list/head), `bucket`*, `key`, `endpoint`, `region`, `content`, `contentType`, `credentials` |
| `chat-completion` | Call an LLM (ChatCompletions spec) | `model`*, `messages`*, `apiUrl`, `apiKey`, `temperature`, `maxTokens`, `topP`, `stream`, `tools`, `responseFormat` |
| `assert` | Assert values (boolean gate) | `assertions`* — each `{ actual, matcher, expected?, not?, message? }` |
| `map` | Bounded fan-out over a collection | `items`*, `task`*, `as`, `concurrency`, `continueOnError` |
| `eval` | Score a subject over a dataset (measure) | `scorers`*, `subject`, `dataset`, `concurrency`, `threshold` |
| `flow` | Import another Flow as a task | `use`* (reference), `inputs` |

<sub>\* required</sub>

**`exec` vs `run`.** `exec` runs a single shell command line. `run` runs a
script/program in a named language or interpreter (`python`, `node`, `ruby`, `bash`, …),
defaulting to `bash`, via either inline `script` or a `file` path.

**Assertions** use [Vitest `expect` matchers](https://vitest.dev/api/expect.html) —
`expect(actual).matcher(expected)`. A representative subset: `toBe`, `toEqual`,
`toContain`, `toMatch`, `toBeGreaterThan`, `toHaveProperty`, `toBeTruthy`,
`toBeInstanceOf`, `toBeCloseTo`, `toBeOneOf`. Set `not: true` to negate a matcher
(Vitest's `.not` modifier). `assert` is a **boolean gate**: the task fails if any assertion
does not hold (aborting the Pipeline unless `continueOnError`). To *measure* quality with
continuous scores over a dataset rather than gate on a single value, use the
[`eval`](#evaluations-eval) task — its scorers reuse these same matchers.

**User-defined tasks** are implemented as Flows imported via the `flow` task type — `use`
references the Flow and `inputs` supplies the imported Flow's declared [input
variables](#flow-inputs).

#### Bounded fan-out (`map`)

The one deliberate exception to sequential-only execution. A `map` task runs a single inner
`task` (a full task object, or a `flow` import for multi-step work) **once per element** of a
collection, then collects the per-element outputs into an **array** under the task's name. The
surrounding task list still runs strictly in order — only the `map` task fans out internally.

| Field | Description |
| --- | --- |
| `items` | **Required.** Array, or a `${{ ... }}` expression resolving to an array. |
| `task` | **Required.** The task/`flow` run for each element. |
| `as` | Alias the current element is bound under inside `task` (default `item`; read as `${{ item.* }}`, index as `${{ item.index }}`). |
| `concurrency` | Max elements processed at once (default `1` = in order). Output array always follows item order. |
| `continueOnError` | `true` records a failing element's error in its slot and continues; `false` (default) aborts on the first failure. |

```yaml
- name: deploy-regions
  type: map
  with:
    items: "${{ inputs.regions }}"
    as: region
    concurrency: 3
    task:
      type: flow
      with:
        use: ./deploy/region.yaml
        inputs: { region: "${{ region }}" }
# → state["deploy-regions"] = [ <output per region>, ... ]
```

This is what makes test matrices and dataset-driven [evals](#evaluations-eval) expressible. It
is **bounded** iteration only — there is still no general branching, DAG, or unbounded
parallelism. See [`docs/examples/map-fanout.yaml`](./docs/examples/map-fanout.yaml).

#### Evaluations (`eval`)

`eval` **measures** the quality of a `subject`'s output against one or more **scorers**,
optionally over a `dataset` of cases, and emits a **scorecard** (per-case scores + aggregate
metrics). It is the *measure* counterpart to `assert`'s *gate*: scores are continuous (`0..1`)
and the task only fails when a `threshold` policy is set and not met.

| Field | Description |
| --- | --- |
| `scorers` | **Required.** One or more [scorers](#scorers) applied to each case's output. |
| `subject` | The task/`flow` under test, run once per case; its output is scored (referenced as `${{ output }}`). Omit to score values already in state. |
| `dataset` | Array of case objects (or a `${{ ... }}`/reference resolving to one); each is bound as `${{ case }}`. Omit to run once. |
| `concurrency` | Max cases evaluated at once (same bounded fan-out as `map`; default `1`). |
| `threshold` | Gating policy `{ metric, min, passScore? }` — without it, `eval` only reports. |

Output is merged under the task name:

```jsonc
{
  "cases":   [ { "case": {…}, "output": …, "scores": { "quality": 0.9 }, "score": 0.9, "passed": true } ],
  "summary": { "mean": 0.86, "weightedMean": 0.88, "passRate": 0.95, "p50": 0.9, "count": 20 },
  "passed":  true
}
```

##### Scorers

Each scorer yields a score in `0..1`; a case's score is the weighted mean of its scorers.
Three kinds, selected by `type`:

| `type` | Scores by | Key fields |
| --- | --- | --- |
| `matcher` (default) | A Vitest matcher → `1.0` if it passes (respecting `not`), else `0.0` | `matcher`, `expected`, `not` |
| `llmRubric` | An LLM judging the output against a rubric (model-graded) | `rubric`, `model`, `apiUrl`, `apiKey` |
| `exec` / `run` | A command/script that emits a number (`{ "score": 0.9 }` or a bare number) | `with` |

Common to all: `name`, optional `actual` (defaults to `${{ output }}`), `weight` (default `1`).
The `matcher` scorer reuses the **same matcher vocabulary as [`assert`](#tasks)** — matchers are
the shared primitive; `assert` consumes them as gates, `eval` consumes them as scorers.

```yaml
- name: grade-assistant
  type: eval
  with:
    dataset: "${{ inputs.cases }}"
    subject:
      type: chat-completion
      with: { model: claude-opus-4-8, messages: [ { role: user, content: "${{ case.input }}" } ] }
    scorers:
      - { name: mentions-expected, matcher: toContain, expected: "${{ case.expected }}" }
      - { name: quality, type: llmRubric, model: claude-opus-4-8,
          rubric: "Correct, concise, polite?", weight: 2 }
    threshold: { metric: weightedMean, min: 0.8 }
```

See [`docs/examples/eval.yaml`](./docs/examples/eval.yaml).

#### Typed task output (`produces`)

Any task may declare a `produces` JSON Schema describing the shape of the JSON it merges into
the Pipeline state. It is **purely declarative** — it has no effect on execution — but it lets a
loader **statically lint** downstream `${{ tasks.NAME.field }}` references before a run (catching
typos like `tasks.build.artifcat`), power editor autocomplete, and optionally check at runtime
that a task's output conforms.

```yaml
- name: build
  type: run
  produces:
    type: object
    required: [artifact, sha]
    properties:
      artifact: { type: string }
      sha:      { type: string }
  with: { language: bash, file: ./build.sh }
# `${{ tasks.build.artifact }}` is now checkable; `${{ tasks.build.artifcat }}` fails lint
```

See [`docs/examples/typed-output.yaml`](./docs/examples/typed-output.yaml).

### Flow inputs

A Flow can declare **input variables** it accepts when invoked — from the CLI or from
another Flow that imports it. Inputs are declared on the Flow under `inputs`, mapping an
input name to its spec:

| Field | Description |
| --- | --- |
| `type` | Optional expected JSON type: `string`, `number`, `boolean`, `object` or `array`. |
| `description` | Optional human description. |
| `required` | `false` (default) or `true` — whether the input must be supplied. |
| `default` | Value used when the input is not supplied (any type). |

```yaml
inputs:
  artifactPrefix:
    type: string
    description: Object-storage key prefix for build artifacts.
    default: builds
```

Inside the Flow, a supplied input is read via interpolation as `${{ inputs.NAME }}`
(e.g. `${{ inputs.artifactPrefix }}`).

**Supplying inputs.** Values are passed as an `inputs` object of name → value:

- **From the CLI** with a repeatable `--input key=value` flag:

  ```bash
  tmx run flow.yaml --input artifactPrefix=releases --input dryRun=true
  ```

- **From a `flow` task** that imports the Flow, via `with.inputs`:

  ```yaml
  - name: deploy
    type: flow
    with:
      use: ../deploy/flow.yaml
      inputs:
        artifact: "${{ inputs.artifactPrefix }}/dist.tgz"
  ```

### Context

The Context is the environment variables and secrets available to a Flow's tasks, plus
lifecycle hooks. Contexts are reusable and can be defined in isolation for reuse across
Flows.

Lifecycle hooks:

| Hook | Runs |
| --- | --- |
| `create` | On Pipeline creation. |
| `change` | Every time the Pipeline state changes. |
| `destroy` | On Pipeline destruction. |
| `error` | To handle errors in the Pipeline. |

A hook body is either a set of tasks defined inline — an ordered array or a name-keyed map
(key = task name), as for a Flow's `tasks` — or a reference to another Flow that
implements it.

**Secrets are opt-in per task.** All secrets are auto-masked in output everywhere; a task
receives a secret in clear text only if it lists that secret's name in its `secrets`
array. A task that names no secrets gets none unmasked. Secret values may be literals,
`${{ ... }}` interpolations, or a structured source (`env` / `file` / `provider` + `key`).

**Inheritance / merge semantics.** When a task defines its own context it is, by default,
merged over the inherited folder/Flow context. Merging is **per-section** —
`env`, `secrets` and `hooks` merge independently at the key level. On a key collision the
in-file (`local`) value wins by default; set `contextPrecedence: inherited` to let the
parent value override instead. `contextStrategy: replace` ignores inheritance and uses
only the task-level context.

### Environment

The Environment declaratively describes the runtime a Flow runs in. It is not required —
Flows run without one — but when present it lets a provider provision the resources for a
Pipeline (a local Docker container, an AWS Lambda, a GCP Kubernetes cluster, …).
Environments are reusable and can be defined in isolation.

Common fields:

| Field | Description |
| --- | --- |
| `os` / `arch` | Operating system / CPU architecture. |
| `platform` | Target platform: `local` or a cloud provider (`aws`, `gcp`, `azure`, `fly`, …). |
| `provider` | The Environment Provider that materialises this environment. |
| `runtime` | `container` \| `vm` \| `microvm` \| `cloud-instance` \| `process`. |
| `image` | Standard image — a container image ref or a machine/VM image id. |
| `resources` | `cpu`, `memory`, `storage`, `gpu`. |
| `bootstrap` | Tasks to run on environment/container init (an inline set of tasks — array or name-keyed map — or a Flow reference). |
| `options` | Provider/platform-specific options (free-form). |

Because environment-specific options are unique to each provider/platform (AWS ECS differs
from AWS EC2, fly.io, Google Cloud Run, …), the Environment object is **open**: unknown
keys are allowed, and a dedicated `options` block carries provider extensions.

#### Environment providers

Environments are materialised by **Environment Providers**, each of which is either:

- a single standalone **binary** that takes an environment definition and deploys it
  (invoked by the core TMX CLI), or
- a **Flow** that implements the standard methods (e.g. a set of CLI calls to stand up the
  environment).

A provider is described by a [provider manifest](./docs/tmx-provider.schema.json)
(`kind: provider`) declaring its `name`, `type` (`binary` | `flow`), an optional `binary`
path, an optional `optionsSchema` (so the CLI can validate an environment's `options`
against the chosen provider), and the four required lifecycle **methods**:

| Method | Responsibility |
| --- | --- |
| `bootstrap` | Bootstrap the environment to enable Flow runs (provision network, create clusters, …). |
| `deploy` | Create what's required for a specific (or set of) Flow runs. |
| `clean` | Remove deployed instances used for Flow runs. |
| `destroy` | Destroy the entire environment, including everything created by `bootstrap`. |

Each method is a subcommand string (binary providers), a Flow reference (or a
`{ use, inputs }` import), or an inline set of TMX tasks — an ordered array or a name-keyed
map, with the same `exec` string shorthand. Because those inline tasks reference the same task
definition as a Flow, a provider can use **every task type** — including
[`map`](#bounded-fan-out-map), [`eval`](#evaluations-eval) and the
[`produces`](#typed-task-output-produces) contract. An environment's `provider` field names the
manifest to use. See
[`docs/examples/provider-manifest.yaml`](./docs/examples/provider-manifest.yaml), whose
`bootstrap` pre-pulls images with a `map` and whose `deploy` declares its container-id output via
`produces`.

## Interpolation

Values may reference secrets, declared [flow inputs](#flow-inputs) and prior Pipeline
state via `${{ ... }}` interpolation (e.g. `Bearer ${{ secrets.API_KEY }}`,
`${{ inputs.artifactPrefix }}`, `${{ tasks.build.success }}`). The schema treats
interpolated values as strings; the expression grammar is evaluated by the engine, not the
schema.

## Schemas & validation

| Path | What |
| --- | --- |
| [`docs/tmx.schema.json`](./docs/tmx.schema.json) | Flow / Task / Context / Environment (JSON Schema Draft 2020-12). |
| [`docs/tmx-provider.schema.json`](./docs/tmx-provider.schema.json) | Environment provider manifest. |
| [`docs/SCHEMA.md`](./docs/SCHEMA.md) | Design decisions, interpretations, and open questions. |
| [`docs/examples/`](./docs/examples) | Worked examples (JSON/YAML/TOML/JSONC) — see its [README](./docs/examples/README.md). |

Validate every schema and example with one script (uses [`uv`](https://docs.astral.sh/uv/)
when available, otherwise a local venv):

```bash
scripts/validate.sh                     # meta-schema + all examples + cross-format parity
scripts/validate.sh path/to/file.yaml   # validate a single file (dispatched by `kind`)
```

### Version control & pre-push

This repo uses **Jujutsu (jj)**, which does not run Git hooks. So:

- **Pushing with jj:** use [`scripts/push.sh`](./scripts/push.sh) — it validates, then runs
  `jj git push`.
- **Plain `git push` / CI:** the `.githooks/pre-push` hook runs the same validation. Enable
  once with `git config core.hooksPath .githooks`.

## Repository layout

```
CHANGELOG.md                 # spec version history (Keep a Changelog)
docs/
  tmx.schema.json            # core schema (Flow/Task/Context/Environment)
  tmx-provider.schema.json   # provider manifest schema
  SCHEMA.md                  # design decisions + open questions
  CLI.md                     # proposed `tmx` command-line interface (design draft)
  RUNTIME.md                 # proposed execution engine — hexagonal ports & adapters (design draft)
  comparison.md              # task/workflow-runner landscape + TMX positioning
  examples/                  # validated examples in all four formats
scripts/
  validate.sh                # validate schemas + examples
  validate_examples.py       # the validator (kind-dispatch, cross-file $ref, parity)
  push.sh                    # jj-native "pre-push": validate then jj git push
.githooks/pre-push           # git pre-push backstop (for `git push` / CI)
```

## License

[MIT](./LICENSE) © 2026 Ant Stanley
