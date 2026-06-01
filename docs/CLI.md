# TMX CLI — Design Proposal

> A proposed command-line interface for `tmx`, the executor the
> [schema](./tmx.schema.json) implies but does not yet describe.
>
> **Status: design draft (CLI v0), no implementation yet.** This document specifies the
> _surface_ — commands, flags, streams, exit codes — for the runtime that will run a
> [Flow](../README.md#flows). It targets **spec version 0.2.0** of
> [`tmx.schema.json`](./tmx.schema.json) and [`tmx-provider.schema.json`](./tmx-provider.schema.json).
> Every command is anchored to a concept that already exists in those schemas; where the CLI
> proposes something beyond the current schema scope (a run store, a name resolver) it is
> flagged under [Design decisions](#design-decisions) and [Open questions](#open-questions).

The design follows the [WebCLI](https://webcli.com/) conventions — the patterns drawn from the
most LLM-saturated CLIs (git, curl, docker, npm, make) so that a model can invoke `tmx`
correctly **without reading this document**.

## Design goals

| Goal                                                                                                                            | WebCLI principle                                 |
| ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **Run a flow with zero ceremony** — `tmx run flow.yaml` is the headline path                                                    | §2 a Pattern-B convenience over a Pattern-A core |
| **LLM-writable without docs** — only canonical verbs (`run`, `validate`, `list`, `init`, `deploy`, `destroy`, `logs`, `status`) | §6 verb vocabulary, §11 transfer learning        |
| **Pipe-clean** — machine data on stdout, all human/progress on stderr                                                           | §4, §5, §10 stream separation                    |
| **One model, four formats** — every command accepts YAML/JSON/JSONC/TOML, dispatched by `kind`                                  | TMX's defining trait                             |
| **Predictable flags** — every flag has a `--long` form, POSIX shorts, `--no-` booleans, both `--flag value` and `--flag=value`  | §1, §7                                           |
| **Scriptable exit codes** — `0` ok, `2` usage, domain codes for run / validation / environment failures                         | §4                                               |
| **Layered config** — flags > `TMX_*` env > project config > user config                                                         | §9                                               |

## Invocation grammar

```
tmx [global flags] <command> [subcommand] [arguments] [command flags]
```

The surface is **hybrid**: high-frequency actions are flat top-level **primaries** (what an LLM
emits first), and resource-heavy areas are grouped under a **noun** (Pattern A).

```
tmx run flow.yaml          # primary
tmx validate flow.yaml     # primary
tmx env deploy flow.yaml   # noun group: env
tmx provider list          # noun group: provider
tmx runs show <id>         # noun group: runs
```

Each primary is sugar for its long form (`tmx run` ≡ `tmx flow run`; `tmx deploy` ≡
`tmx env deploy`). Both spellings always work.

## Schema concept → command

The CLI is a direct projection of the data model. Every command resolves to a schema concept:

| Schema concept                                     | Command(s)                                                    |
| -------------------------------------------------- | ------------------------------------------------------------- |
| **Flow** (`$defs/flow`)                            | `tmx run`, `tmx inspect`, `tmx lint`, `tmx init`, `tmx list`  |
| **Task** (`$defs/task`)                            | `tmx run --only/--from/--until`, `tmx list tasks`             |
| **Pipeline** (runtime state)                       | `tmx run` (produces it), `tmx runs …` (inspects it)           |
| **Flow `inputs`** (`$defs/inputSpec`)              | `tmx run --input`, `tmx list inputs`                          |
| **Context** (`$defs/context`)                      | `tmx context show`, `tmx run --env`                           |
| **Secrets** (`context.secrets`)                    | `tmx secrets list` (masked), `secretSource` resolution at run |
| **Environment** (`$defs/environment`)              | `tmx env …`, `tmx run` lifecycle                              |
| **Provider manifest** (`tmx-provider.schema.json`) | `tmx provider …`, and the `env` methods it backs              |
| **All artifacts** (`kind`-dispatch)                | `tmx validate`, `tmx fmt`                                     |

## Command map

| Command                                                         | Purpose                                                                                               |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| **`tmx run [flow]`**                                            | Execute a flow; the headline command                                                                  |
| `tmx validate [file…]`                                          | JSON-Schema validate, `kind`-dispatched (a port of `scripts/validate.sh`)                             |
| `tmx lint [flow]`                                               | Static analysis: resolve references, check `${{ }}` targets against `produces`, secrets, flow imports |
| `tmx inspect [flow]`                                            | Show the _resolved_ flow: merged environment + context, ordered task plan, inputs, secrets needed     |
| `tmx init [name]`                                               | Scaffold a starter flow (single-file or folder-layout)                                                |
| `tmx fmt [file…]`                                               | Canonicalize / convert between YAML · JSON · JSONC · TOML                                             |
| `tmx list [kind] [flow]`                                        | Discover flows / tasks / inputs / providers                                                           |
| `tmx env <up\|down\|bootstrap\|deploy\|clean\|destroy\|status>` | Provision / tear down the runtime substrate                                                           |
| `tmx provider <list\|inspect\|add\|validate>`                   | Manage Environment Providers                                                                          |
| `tmx runs <list\|show\|state\|logs\|rm>`                        | Inspect past / active pipeline runs                                                                   |
| `tmx context show [flow]`                                       | Show resolved `env` + (masked) secrets                                                                |
| `tmx secrets list [flow]`                                       | List secret names + their source (values masked)                                                      |
| `tmx version` / `tmx help [command]`                            | Standard                                                                                              |

## Global flags

Present on the root and every subcommand (WebCLI §1):

| Flag                              | Short | Effect                                                             |
| --------------------------------- | ----- | ------------------------------------------------------------------ |
| `--help`                          | `-h`  | Help for the tool or a subcommand (`tmx run --help`)               |
| `--version`                       |       | Prints CLI + supported spec version, e.g. `tmx 0.1.0 (spec 0.2.0)` |
| `--verbose`                       | `-v`  | More detail to stderr; stackable (`-vv`)                           |
| `--quiet`                         | `-q`  | Suppress stderr progress; stdout data is unaffected                |
| `--directory <dir>`               | `-C`  | Run as if started in `<dir>` (git / make convention)               |
| `--file <path>`                   | `-f`  | Explicit artifact; overrides discovery                             |
| `--dry-run`                       | `-n`  | Plan only — resolve + validate, execute nothing                    |
| `--format <pretty\|json\|ndjson>` |       | Output rendering; `pretty` on a TTY, `json` when piped             |
| `--color` / `--no-color`          |       | Force colour on/off; honours `NO_COLOR`                            |
| `--config <path>`                 |       | Explicit config file                                               |
| `--profile <name>`                |       | Named config profile                                               |

Flag conventions (WebCLI §7): every flag has a `--long` form; booleans negate with `--no-`
(`--no-color`, `--no-env`); value flags accept both `--flag value` and `--flag=value`; POSIX
shorts stack (`-qv`).

## `tmx run` — the primary command

```
tmx run [flow] [flags]
```

### Flow resolution

`flow` may be a **file path** (any of the four formats), a **directory**, or a **registered
name**. When omitted, `tmx` discovers it in order:

1. `--file/-f`, else the positional argument
2. `$TMX_FLOW`
3. `./flow.{yaml,yml,json,jsonc,toml}`, then `./tmx.{…}`
4. a folder-layout in the cwd (`environment.*` + `context.*` + any task files)
5. otherwise → exit `4` (resolution error), printing the search path to stderr

**Directory argument.** `tmx run <dir>` runs **every task in the directory** as one sequential flow,
sharing the folder-level `environment.*` and `context.*` when present (same-folder only, per the
schema). Task files may use **any filename** — the `task-N.*` convention is illustrative, not
required. A file is treated as a task if its `kind` is `task`, or (when `kind` is omitted) if it is
not one of the reserved `environment.*` / `context.*` / `flow.*` artifacts; tasks run in **natural
filename order** (so `task-2` precedes `task-10`, `build` before `deploy`). **Every task file is
validated against the task schema before the run starts** — a validation failure aborts the whole
run (exit `3`) before any task executes, so a folder never runs half-way on a malformed task.

A standalone **task file** (any name) is likewise validated, then wrapped into a one-task flow and
run.

> **Registered names** (`my-org/base-context`, and the same for environments, flows and providers)
> are accepted by the schema's `reference` type, but **v0 resolves references as file paths only**;
> named-registry resolution is deferred (see [Open questions](#open-questions)). The one concrete
> registry kept in v0 is the local provider map populated by `tmx provider add`.

### Run flags

| Flag                           | Short | Effect                                                                                                    |
| ------------------------------ | ----- | --------------------------------------------------------------------------------------------------------- |
| `--input key=value`            | `-i`  | Supply a declared flow input; repeatable. Coerced to the input's declared `type`                          |
| `--input key:=<json>`          |       | Raw-JSON input (httpie convention) for `object` / `array` inputs                                          |
| `--inputs-file <path>`         |       | Bulk inputs from a JSON / YAML file (merged under `--input`)                                              |
| `--env KEY=VALUE`              | `-e`  | Override a context `env` var; repeatable                                                                  |
| `--state-in <path>`            |       | Seed the Pipeline state (resume / fixture)                                                                |
| `--state-out <path>`           |       | Also write the final Pipeline state JSON to a file                                                        |
| `--only NAME[,…]`              |       | Run only these tasks                                                                                      |
| `--skip NAME[,…]`              |       | Skip these tasks                                                                                          |
| `--from NAME` / `--until NAME` |       | Run a contiguous slice of the sequential task list                                                        |
| `--dry-run`                    | `-n`  | Resolve + validate + print the ordered plan; execute nothing                                              |
| `--local` / `--no-env`         |       | Ignore the declared `environment`; run in the current process                                             |
| `--keep`                       |       | Leave the provisioned environment up after the run (see [lifecycle](#how-run-relates-to-the-environment)) |
| `--no-deploy`                  |       | Reuse a standing environment (skip `deploy`/`clean`)                                                      |
| `--concurrency N`              |       | Global cap for `map` / `eval` fan-out                                                                     |
| `--matrix key=v1,v2,…`         |       | Desugar to a bounded `map` over the values; repeatable axes form a cross-product (`${{ matrix.key }}`)     |
| `--continue-on-error`          |       | Force the envelope flag across all tasks                                                                  |
| `--check-produces=strict`      |       | Runtime-check each task output against its `produces` schema and fail on mismatch (off by default)        |
| `--timeout <dur>`              |       | Global wall-clock budget (`30s`, `5m`)                                                                    |
| `--max-state-size <size>`      |       | Adjust the in-memory Pipeline-state cap (default `512MiB`); exceeding it aborts the run                   |
| `--no-store`                   |       | Do not record this run (see [Pipeline runs](#pipeline-runs))                                              |
| `--watch`                      | `-w`  | Re-run on file change (dev loop)                                                                          |

`--only`/`--from`/`--until` slice the (sequential) task list; because later tasks read prior
Pipeline state via `${{ tasks.NAME.field }}`, pair a slice with `--state-in` to supply the
state earlier tasks would have produced.

### Inputs on the CLI

Inputs are declared with a `type` in the Flow; the CLI coerces accordingly:

```bash
tmx run flow.yaml \
  --input artifactPrefix=releases \   # string
  --input dryRun=true \               # boolean — coerced per declared type
  --input retries=3 \                 # number
  --input 'regions:=["us-east-1","eu-west-1"]'   # array — raw JSON via :=
```

Unknown inputs, or a value that fails coercion to the declared `type`, → exit `4`. A `required`
input with no value → exit `4` and the missing names on stderr.

### Matrix sugar

The model expresses one bounded axis with the `map` task. `--matrix` is CLI sugar that desugars to
a `map`, so you don't have to hand-write one for an ad-hoc sweep:

```bash
tmx run eval.yaml --matrix model=opus,sonnet,haiku                # one axis  → 3 runs
tmx run eval.yaml --matrix model=opus,sonnet --matrix temp=0,1    # two axes  → cross-product, 4 runs
```

Each `--matrix key=v1,v2,…` adds an axis; multiple axes form the **cross-product**. Each
combination binds `${{ matrix.<key> }}` and runs the flow's task list once; results collect into an
array under the generated `map`, exactly as if authored. `--concurrency` bounds how many
combinations run at once. It is sugar only — a flow that needs finer control writes an explicit
`map`.

**Precedence — an authored `map` wins.** If the target flow already contains a `map` task,
`--matrix` is **ignored** (the in-flow iteration is authoritative) and `tmx run` warns on stderr.
`--matrix` is for flows that don't already express their own fan-out; once a flow authors a `map`,
that definition governs and the CLI never rewrites or wraps it.

### Spec-version compatibility

Each schema pins a spec version in its `$id` path (currently `0.2.0`); `tmx --version` reports the
spec version the CLI supports. When a Flow targets a **newer** spec, `tmx run` **warns on stderr
but still runs**, as long as the document only uses constructs the CLI understands — it fails (exit
`3`) only when it hits a task type, field, or `with` shape it cannot interpret. An older spec always
runs. (A Flow does not declare a version itself — the loader selects the schema — so this compares
the features used, not a document field.)

### stdout / stderr contract

- **stdout** → the **final Pipeline state** as one JSON object, secrets masked. `tmx run` is
  pipe-first: `tmx run flow.yaml | jq '.build.sha'` just works.
- **stderr** → human progress: per-task start/finish, timings, skips (`if` false), masked-secret
  notices, and the `eval` scorecard summary.
- `--format ndjson` → stream **one JSON event per line** to stdout — ideal for CI and programmatic
  / LLM consumers:

  ```jsonc
  {"event":"run.start","flow":"build-and-publish","id":"r_01H…"}
  {"event":"task.finish","name":"build","status":"ok","ms":812,"output":{"artifact":"dist.tgz","sha":"a1b2c3"}}
  {"event":"task.skip","name":"upload","reason":"if=false"}
  {"event":"run.finish","status":"ok","ms":2042}
  ```

- `--format pretty` (the TTY default) → a human run summary on stdout; `--quiet/-q` reduces output
  to the final state only.

This is the WebCLI §5 / §10 contract, and it directly enables the **LLM-eval-as-CI-gate** wedge
from [`comparison.md`](./comparison.md#9-where-tmx-is-the-clear-choice): an `eval` whose
`threshold` is not met fails the task, which exits non-zero and blocks the merge.

```bash
tmx run eval.yaml -i model=claude-opus-4-8   # exit 1 if the eval threshold isn't met
```

## Authoring & validation

```bash
tmx validate flow.yaml                 # schema-validate one artifact (kind-dispatched)
tmx validate 'docs/examples/**'        # many; exit 3 on any failure
tmx lint flow.yaml                     # references resolve? ${{ tasks.build.artifcat }} typo? undeclared secret?
tmx lint flow.yaml --strict            # warnings become errors
tmx inspect flow.yaml                  # resolved env + context, ordered plan, inputs, secrets-needed
tmx inspect flow.yaml --format json    # machine-readable resolution
tmx init release --template eval --layout folder
tmx fmt flow.toml --to yaml --write    # convert TOML → YAML in place (one model, four formats)
tmx list                               # flows discovered in the cwd
tmx list inputs flow.yaml              # declared inputs + types + defaults + required
tmx list tasks flow.yaml               # the ordered task list
```

- **`validate`** is pure JSON-Schema, the same `kind`-dispatch as
  [`scripts/validate.sh`](../scripts/validate.sh). Exit `0` / `3`.
- **`lint`** is where the schema doc's promised _static `produces` checking_ lives: it resolves
  `environment` / `context` / `flow` references, walks `${{ tasks.NAME.field }}` against each
  task's `produces` schema (catching `tasks.build.artifcat`), flags inputs used-but-undeclared,
  secrets used-but-not-listed in a task's `secrets`, and cyclic `flow` imports. Exit `0` / `3`.
- **`inspect`** is the static resolution view (read-only); `run --dry-run` is the _execution_
  plan. They overlap deliberately — `inspect` answers "what is this flow," `run -n` answers "what
  would this run do."
- **`fmt --to`** is loss-free because all four source formats parse to the same JSON model.
- **`init --template`** ships starters for the documented use cases (`ci`, `eval`, `deploy`,
  `minimal`); `--layout single|folder` chooses a single combined file or the standalone
  folder-layout.

## Environment & providers

A Flow's `environment` names a `provider`, whose [manifest](./tmx-provider.schema.json) declares
four lifecycle methods. The CLI exposes them **1:1**, plus `up` / `down` aggregates:

| Command                    | Provider method(s)     | Meaning                                       |
| -------------------------- | ---------------------- | --------------------------------------------- |
| `tmx env bootstrap [flow]` | `bootstrap`            | Provision shared substrate (network, cluster) |
| `tmx env deploy [flow]`    | `deploy`               | Create resources for a run                    |
| `tmx env clean [flow]`     | `clean`                | Remove per-run resources                      |
| `tmx env destroy [flow]`   | `destroy`              | Tear down everything `bootstrap` created      |
| `tmx env up [flow]`        | `bootstrap` + `deploy` | Stand up and leave running                    |
| `tmx env down [flow]`      | `clean` + `destroy`    | Full teardown                                 |
| `tmx env status [flow]`    | —                      | Show current substrate state                  |

```bash
tmx provider list                      # registered providers
tmx provider inspect aws-ecs           # manifest, methods, optionsSchema
tmx provider validate ./provider.yaml  # validate a kind:provider manifest
tmx provider add ./aws-ecs.yaml        # register a manifest / binary
```

For a `binary` provider the CLI invokes the executable with the method's subcommand string; for a
`flow` provider it runs the method's inline tasks / referenced Flow through the same engine as
`tmx run`. `tmx provider validate` checks the manifest, and — when `optionsSchema` is present —
`tmx lint`/`tmx validate` checks an environment's `options` against the chosen provider.

### Two lifecycles, kept distinct

- The **provider** lifecycle (`bootstrap` / `deploy` / `clean` / `destroy`) is the _substrate_ and
  is driven by `tmx env …`.
- The **context** lifecycle hooks (`create` / `change` / `destroy` / `error`) are the _pipeline_
  and fire **inside** `tmx run`, driven by the engine as state changes — not by separate commands.
  (`change` fires at the end of each task only when the state actually changed, per
  [`SCHEMA.md`](./SCHEMA.md#still-open).)

### How `run` relates to the environment

`tmx run` provisions **ephemerally** by default — a clean-room run:

```
tmx run flow.yaml              → deploy → run pipeline → clean        (substrate assumed bootstrapped)
tmx run flow.yaml --keep       → deploy → run                         (leave it up)
tmx run flow.yaml --no-deploy  → run only                             (reuse a standing env)
tmx run flow.yaml --local      → run in the current process           (ignore environment entirely)

tmx env up flow.yaml           → bootstrap + deploy
tmx env down flow.yaml         → clean + destroy
```

A Flow with **no** `environment` always runs locally — the block is optional in the schema. For a
fast dev loop, `--local` skips provisioning; for CI, the ephemeral default gives reproducible
clean-room runs.

## Pipeline runs

The schema declares the runtime Pipeline "out of scope." The CLI **is** that runtime, so it records
each run to a lightweight local store at `./.tmx/runs/<id>/` (a final-state snapshot + the ndjson
event log), where `<id>` is a **UUIDv7** — time-ordered, so `tmx runs list` is naturally
chronological without a separate timestamp sort. This is a **record, not a journal** — durability /
replay stays explicitly out of scope.

```bash
tmx runs list                         # recent runs: id, flow, status, duration, started
tmx runs show <id>                    # final state + per-task result and timing
tmx runs state <id>                   # dump the Pipeline state JSON (pipe to jq)
tmx runs logs <id> [--task N]         # the event log (secrets masked)
tmx runs prune [--older-than <dur>]   # delete aged runs (default: 30 days)
tmx runs rm <id|--all>
```

**Retention.** Stored runs are purged after a default **30 days** — applied opportunistically at
the start of each `tmx run` and on demand via `tmx runs prune`. Override with `--older-than`, the
`runs.retention` project-config key, or `TMX_RUNS_RETENTION` (`0` / `off` disables purging).
`tmx run --no-store` opts out of recording entirely.

## Context & secrets

```bash
tmx context show flow.yaml     # resolved env + secret names (all masked), merged per inheritance
tmx secrets list flow.yaml     # secret names + their source (env/file/provider), values masked
```

Secrets resolve through the context's `secretSource` (`env` / `file` / `provider` + `key`) and are
**auto-masked everywhere** — in logs, the final state, run records, and these commands. They are
**never** accepted as plain CLI flags (a process-table leak); for ad-hoc injection, set a host env
var and reference it from a `secretSource: { env: … }`. Per the schema, a task receives a secret
unmasked only if it names it in its `secrets` array.

## Output formats

| `--format`             | stdout                                                                     | Use                                         |
| ---------------------- | -------------------------------------------------------------------------- | ------------------------------------------- |
| `pretty` (TTY default) | human run summary                                                          | Interactive                                 |
| `json` (pipe default)  | final Pipeline state, one object                                           | Pipe to `jq`, capture state                 |
| `ndjson`               | one event per line (`run.start`, `task.finish`, `task.skip`, `run.finish`) | CI, streaming, programmatic / LLM consumers |

Data always goes to **stdout**; logs, progress and warnings always go to **stderr** (WebCLI §4).

## Exit codes

| Code  | Meaning                                                                                               |
| ----- | ----------------------------------------------------------------------------------------------------- |
| `0`   | Success — all tasks ran, every `assert` held, every `eval` threshold met                              |
| `1`   | **Run failure** — a task aborted the Pipeline, an `assert` failed, or an `eval` threshold was not met |
| `2`   | **Usage error** — unknown command, bad flag or argument                                               |
| `3`   | **Validation error** — an artifact failed schema validation or `lint`                                 |
| `4`   | **Resolution error** — flow / reference / provider not found, or a bad `${{ }}` / input               |
| `5`   | **Environment error** — a provider method (`bootstrap`/`deploy`/`clean`/`destroy`) failed             |
| `124` | Timeout (`--timeout` exceeded)                                                                        |
| `130` | Interrupted (SIGINT)                                                                                  |

`0` and `2` match WebCLI exactly; the domain codes let CI branch on _why_ a run failed — a failed
eval gate (`1`) is not a broken file (`3`) is not a provider outage (`5`).

## Configuration

Resolved highest-to-lowest priority (WebCLI §9):

1. **CLI flags**
2. **`TMX_*` environment variables** — `TMX_FORMAT`, `TMX_CONCURRENCY`, `TMX_FLOW`, `TMX_PROFILE`,
   `TMX_NO_ENV=1`, `TMX_NO_COLOR`; inputs via `TMX_INPUT_<NAME>` (e.g.
   `TMX_INPUT_ARTIFACTPREFIX=releases`)
3. **Project config** — `tmx.config.{toml,yaml,json,jsonc}` in the project root: flag defaults,
   registered-name → path mappings, the provider registry, and named profiles
4. **User config** — `~/.config/tmx/config.toml`
5. **System config** — `/etc/tmx/config.toml`

## Worked examples

```bash
# Run, pipe the merged state
tmx run flow.yaml -i artifactPrefix=releases | jq '.upload.status'

# Local dev: watch + pretty
tmx run --local --watch

# CI eval gate — a missed threshold exits non-zero and blocks the merge
tmx run eval.yaml --format ndjson 1> events.ndjson

# Validate + lint everything in CI (replaces scripts/validate.sh)
tmx validate 'docs/examples/**' && tmx lint flow.yaml --strict

# Provision once, run reusing the standing env, then tear down
tmx env up prod-flow.yaml
tmx run prod-flow.yaml --no-deploy -i version=1.4.2
tmx env down prod-flow.yaml

# Convert a flow between source formats (one model, four formats)
tmx fmt flow.toml --to yaml --write

# Scaffold a new folder-layout flow from the deploy template
tmx init release --template deploy --layout folder

# Inspect what a flow needs before running it
tmx inspect flow.yaml          # plan, inputs, secrets
tmx list inputs flow.yaml      # just the declared inputs
```

## Design decisions

Interpretations made while drafting this surface. Each records the chosen path and its
alternative, in the style of [`SCHEMA.md`](./SCHEMA.md#design-decisions).

1. **Hybrid command surface.** Flat primaries for the high-frequency verbs (`run`, `validate`,
   `lint`, `init`, `fmt`, `inspect`) plus noun groups for resource areas (`env`, `provider`,
   `runs`). _Chosen_ over strict verb-noun (more uniform, but `tmx flow run` buries the headline)
   and flat-verbs-only (shortest, but verbs collide as the surface grows). Primaries are sugar for
   the long form, so both spellings work.
2. **`tmx run` provisions ephemerally.** `deploy → run → clean` around each run by default;
   `--keep`, `--no-deploy`, and `--local` adjust it. _Chosen_ over explicit-only (every run needs
   a prior `tmx env up`) and local-by-default (the declared environment is ignored unless opted
   in). Ephemeral gives reproducible CI runs while `--local` keeps the dev loop fast.
3. **stdout is the final Pipeline state (JSON).** Progress goes to stderr; the merged state object
   goes to stdout so `tmx run | jq` works without flags. _Chosen_ over a human-summary default
   (needs a flag to script) and a quiet default (needs a flag for the common case). `--format
pretty` restores the human view; `--format ndjson` streams events.
4. **`validate` vs `lint` are split.** `validate` is pure JSON-Schema (`kind`-dispatch, a port of
   `scripts/validate.sh`); `lint` adds reference resolution and the `produces`-based interpolation
   checking the schema docs promise. Two exit lanes (`3` for both) but distinct depth.
5. **Provider methods map 1:1 to `tmx env` subcommands**, with `up`/`down` as aggregates — so the
   CLI never invents a lifecycle the provider manifest doesn't define.
6. **No overloaded `-o`; redirect instead.** WebCLI lists both `-o FILE` and `--output json`; to
   avoid the collision the CLI has **no `-o` flag** — `--format` selects the renderer and shell
   redirection (`> file`) writes it anywhere, with `--state-out` as the one explicit file
   convenience (it pairs with `--state-in`).
7. **Runs get UUIDv7 IDs and a 30-day retention.** The local run store keys each run by a
   time-ordered UUIDv7 (so listings are chronological) and purges records older than 30 days —
   opportunistically on `tmx run`, on demand via `tmx runs prune`, configurable (`0`/`off`
   disables). It stays a record, not a durable journal.
8. **`--matrix` sugar desugars to `map`; an authored `map` takes precedence.** Repeatable
   `--matrix key=v1,v2` axes form a cross-product that lowers to the bounded `map` task and binds
   `${{ matrix.key }}` — convenience for ad-hoc sweeps without adding a new construct to the
   sequential-plus-bounded-`map` model. When the flow already contains a `map`, that authored
   iteration wins and `--matrix` is ignored (with a stderr warning) — the CLI never rewrites or
   wraps an explicit `map`. _Chosen_ over composing the two (the authored definition is the source
   of truth, and silent wrapping would surprise).
9. **A directory argument runs all its tasks.** `tmx run <dir>` runs every task file (any filename;
   `kind: task` or simply not a reserved `environment.*`/`context.*`/`flow.*` artifact) in natural
   filename order as one sequential flow, sharing a sibling `environment.*`/`context.*` when present.
   Each task is **validated before the run starts**, so a malformed task aborts (exit `3`) before
   anything executes — the folder-layout, generalised to any directory and any task names.
10. **Newer-spec Flows warn but run.** `tmx run` proceeds on a Flow pinned to a newer spec version
    as long as every construct it uses is understood, failing only on an unknown one — forward-
    tolerant rather than silently misreading or hard-refusing.
11. **Registered-name resolution is deferred.** v0 resolves `reference` strings as file paths only;
    remote/namespaced names (`my-org/base-context`) wait for a resolver spec. The sole v0 registry
    is the local provider map from `tmx provider add`.

## Open questions

The CLI v0 review resolved the run-store lifecycle (UUIDv7 IDs + 30-day retention), `-o` vs
redirection, `--matrix` sugar and its precedence under an authored `map`, the directory-run
argument, and newer-spec tolerance — all folded into [Design decisions](#design-decisions) 6–11
above. What remains open:

- **Registered-name resolver (deferred).** The schema's `reference` type allows registered names
  (`my-org/base-context`) for any reusable artifact — context, environment, flow, provider — but v0
  resolves references as **file paths only**. A naming scheme, a registry/index, and
  `add`/`pull` semantics are deferred to a later spec; the only v0 registry is the local provider
  map. (`tmx provider add` registers a local name → path mapping, not a remote namespace.)

## Related

- [`RUNTIME.md`](./RUNTIME.md) — the execution engine that backs these commands (hexagonal ports & adapters)
- [`README.md`](../README.md) — the TMX model (Flow / Pipeline / Task / Context / Environment / Provider)
- [`SCHEMA.md`](./SCHEMA.md) — schema design decisions and open questions
- [`tmx.schema.json`](./tmx.schema.json) · [`tmx-provider.schema.json`](./tmx-provider.schema.json) — the schemas this CLI projects
- [`comparison.md`](./comparison.md) — landscape positioning (the CLI realises the wedges in §9)
