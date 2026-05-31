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
filename convention. It is optional, so a minimal `{ tasks: [...] }` file is still valid.

## Flows

Flows are defined as a hierarchy of Environment → Context → Tasks. Only the tasks are
required. Environment, Context and Tasks can be defined within a single file as separate
sections, **or** as standalone files in a folder used as default configs with inheritance:

```
<flow_folder>
   |
   |- environment.[yaml|json|jsonc|toml]   <- where tasks run. Not required.
   |- context.[yaml|json|jsonc|toml]       <- shared env vars, secrets, lifecycle hooks.
   |- task-1.[yaml|json|jsonc|toml]        <- inherits the folder environment + context.
   |- task-2.[yaml|json|jsonc|toml]
   |- task-3 ...
```

In this layout all tasks inherit the environment and context from the standalone files.
Where an inline artifact and a `reference` are both allowed, a **reference** is just a
string — a relative/absolute file path or a registered name (e.g. `./context.yaml` or
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

Tasks run **in sequence**, one after another. There is no branching, looping or parallel
execution — the only control flow is skipping a task via its `if` condition.

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
| `chat-completion` | Call an LLM (ChatCompletions spec) | `model`*, `messages`*, `apiUrl`, `baseUrl`, `apiKey`, `temperature`, `maxTokens`, `topP`, `stream`, `tools`, `responseFormat` |
| `assert` | Assert values | `assertions`* — each `{ actual, operator, expected?, message? }` |
| `flow` | Import another Flow as a task | `use`* (reference), `inputs` |

<sub>\* required</sub>

**`exec` vs `run`.** `exec` runs a single shell command line. `run` runs a
script/program in a named language or interpreter (`python`, `node`, `ruby`, `bash`, …),
defaulting to `bash`, via either inline `script` or a `file` path.

**Assertions** use a structured operator set: `eq`, `ne`, `gt`, `gte`, `lt`, `lte`,
`contains`, `matches`, `exists`, `notExists`.

**User-defined tasks** are implemented as Flows imported via the `flow` task type — `use`
references the Flow and `inputs` supplies the imported Flow's declared [input
variables](#flow-inputs).

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

A hook body is either a set of tasks defined inline, or a reference to another Flow that
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
| `bootstrap` | Tasks to run on environment/container init (an inline task list or a Flow reference). |
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

Each method is a subcommand string (binary providers), a Flow reference, or an inline list
of TMX tasks. An environment's `provider` field names the manifest to use. See
[`docs/examples/provider-manifest.yaml`](./docs/examples/provider-manifest.yaml).

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
docs/
  tmx.schema.json            # core schema (Flow/Task/Context/Environment)
  tmx-provider.schema.json   # provider manifest schema
  SCHEMA.md                  # design decisions + open questions
  examples/                  # validated examples in all four formats
scripts/
  validate.sh                # validate schemas + examples
  validate_examples.py       # the validator (kind-dispatch, cross-file $ref, parity)
  push.sh                    # jj-native "pre-push": validate then jj git push
.githooks/pre-push           # git pre-push backstop (for `git push` / CI)
```

## License

[MIT](./LICENSE) © 2026 Ant Stanley
