# 07 — CLI Surface

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley

The `tmx` binary is the driving adapter: it maps commands to use cases, parses arguments with `clap`,
resolves layered configuration, selects reporters, and — uniquely — maps core
[`ErrorCategory`](canonical-types.schema.json) to process exit codes. It is a thin shell over the use
cases; no business logic lives here.

This page is the Rust-implementation view of [`CLI.md`](../CLI.md), which is the authoritative surface
design (commands, flags, rationale). Where they overlap, `CLI.md` is the source for *what the surface
is*; this page records *how the binary realises it* and the use-case mapping.

---

## Command → use case mapping

Every command is a thin call into a driving-port use case in `tmx-core::ports::driving`:

| Command | Use case | Notes |
|---|---|---|
| `tmx run [flow]` | `RunFlow(ref, inputs, opts)` | load → resolve → (provision) → `PipelineRunner` → report → store |
| `tmx validate [file…]` | `ValidateArtifacts(paths)` | `SchemaValidator`, `kind`-dispatch |
| `tmx lint [flow]` | `LintFlow(ref)` | resolution + `produces`/secret/import static checks |
| `tmx inspect [flow]` | `InspectFlow(ref)` | resolved env+context, ordered plan, inputs, secrets-needed |
| `tmx init [name]` | `ScaffoldFlow(template, layout)` | starter Flow (single-file or folder) |
| `tmx fmt [file…]` | `FormatArtifact(path, to)` | `SourceLoader` → re-emit; loss-free across the four formats |
| `tmx list [kind] [flow]` | `Discover(kind, ref)` | flows / tasks / inputs / providers |
| `tmx env <…>` | `ProvisionEnvironment(ref, method)` | provider methods 1:1 + `up`/`down` aggregates |
| `tmx provider <…>` | `ManageProviders(op)` | registry read/write, manifest validation |
| `tmx runs <…>` | `QueryRuns(op)` | `RunStore` list/show/state/logs/prune/rm |
| `tmx context show` / `tmx secrets list` | `InspectFlow` (projection) | resolved env + masked secrets |
| `tmx version` / `tmx help` | — (CLI-local) | prints CLI + supported spec version |

The surface is **hybrid**: high-frequency actions are flat primaries (`run`, `validate`, `lint`,
`init`, `fmt`, `inspect`) and resource areas are noun groups (`env`, `provider`, `runs`). A primary is
sugar for its long form (`tmx run` ≡ `tmx flow run`; both always work).

---

## `tmx run` — the primary command

Flow resolution order (when `[flow]` is omitted): `--file/-f` → positional → `$TMX_FLOW` →
`./flow.{yaml,yml,json,jsonc,toml}` then `./tmx.{…}` → a folder-layout in the cwd → else
`ResolutionError` (exit 4) printing the search path. A **directory** argument runs every task file in
natural filename order as one sequential Flow (see [03](03-loading-and-preflight.md#directory-assembly)).

The run flags realise [`CLI.md` §Run flags](../CLI.md#run-flags); the load-bearing ones for the engine:

| Flag | Effect | Engine touchpoint |
|---|---|---|
| `--input k=v` / `k:=<json>` / `--inputs-file` | supply declared inputs; coerced to declared `type` | `Scope.inputs` |
| `--env K=V` | override a context `env` var | resolved context |
| `--state-in` / `--state-out` | seed / dump the Pipeline state | `PipelineState` |
| `--only` / `--skip` / `--from` / `--until` | slice the sequential task list | the task loop |
| `--dry-run` / `-n` | resolve + validate + print the plan; execute nothing | preflight only |
| `--local` / `--no-env`, `--keep`, `--no-deploy` | environment lifecycle | `EnvironmentProvider` |
| `--concurrency N` | global cap for `map`/`eval` fan-out | `Scheduler` |
| `--matrix k=v1,v2` | desugar to a bounded `map`; cross-product across axes | `map` lowering |
| `--continue-on-error` | force the envelope flag across all tasks | error policy |
| `--timeout <dur>` | global wall-clock budget | `Clock` + cancellation |
| `--max-state-size <size>` | adjust `STATE_SIZE_MAX_BYTES` | state cap |
| `--check-produces=strict` | runtime `produces` conformance | `produces` check |
| `--no-store` | do not record this run | `RunStore` |

Slicing (`--only`/`--from`/`--until`) pairs with `--state-in`, since later tasks read prior state via
`${{ tasks.NAME.field }}`.

### Matrix sugar

`--matrix key=v1,v2,…` is CLI sugar that lowers to a bounded `map`; repeatable axes form a
cross-product, each combination binding `${{ matrix.<key> }}`. **An authored `map` wins**: if the
target Flow already contains a `map`, `--matrix` is ignored and `tmx run` warns on stderr — the CLI
never rewrites or wraps an explicit `map`.

---

## stdout / stderr contract

The WebCLI stream separation, implemented by the reporter adapters
([`EventSink`](08-errors-and-observability.md#events--reporters)):

- **stdout** → machine data: the **final Pipeline state** as one JSON object (secrets masked), so
  `tmx run flow.yaml | jq '.build.sha'` works without flags.
- **stderr** → human progress: per-task start/finish, timings, skips, masked-secret notices, the
  `eval` scorecard summary.
- `--format ndjson` → one JSON [`Event`](canonical-types.schema.json) per line to stdout — for CI and
  programmatic/LLM consumers.

| `--format` | stdout | Use |
|---|---|---|
| `pretty` (TTY default) | human run summary | interactive |
| `json` (pipe default) | final Pipeline state, one object | pipe to `jq`, capture state |
| `ndjson` | one event per line | CI, streaming, programmatic |

All three pass through the Masker; `--format` selects the stdout reporter, stderr progress is
independent.

---

## Configuration

Resolved highest-to-lowest (the composition root reads this before constructing adapters):

1. **CLI flags**
2. **`TMX_*` environment** — `TMX_FORMAT`, `TMX_CONCURRENCY`, `TMX_FLOW`, `TMX_PROFILE`, `TMX_NO_ENV`,
   `TMX_NO_COLOR`, `TMX_MAX_STATE_SIZE`, `TMX_RUNS_RETENTION`; inputs via `TMX_INPUT_<NAME>`
3. **Project config** — `tmx.config.{toml,yaml,json,jsonc}` in the project root: flag defaults,
   registered-name → path mappings, the provider registry, named profiles
4. **User config** — `~/.config/tmx/config.toml`
5. **System config** — `/etc/tmx/config.toml`

`config.rs` resolves the layers into one effective config struct; `compose.rs` consumes it.

---

## Pipeline runs

The schema declares the runtime Pipeline "out of scope"; the CLI **is** that runtime, so `RunFlow`
records each run via the `RunStore` port to `./.tmx/runs/<id>/` (a final-state snapshot + the ndjson
event log), keyed by a **UUIDv7** so `tmx runs list` is chronological without a sort key. It is a
**record, not a journal** — no replay/durability. Records purge after a default **30 days**
(opportunistic on `tmx run`, on demand via `tmx runs prune`; `runs.retention` / `TMX_RUNS_RETENTION`,
`0`/`off` disables). `--no-store` opts out. See [08](08-errors-and-observability.md#run-store).

---

## Exit codes

The CLI adapter is the **only** code that maps a core error category to a process exit code:

| Code | Meaning | Core category |
|---|---|---|
| `0` | Success — all tasks ran, every `assert` held, every `eval` threshold met | — |
| `1` | Run failure — a task aborted, an `assert` failed, an `eval` threshold missed, state cap exceeded | `run_failure` |
| `2` | Usage error — unknown command, bad flag/argument | *(CLI-local, not a core category)* |
| `3` | Validation error — an artifact failed schema validation or `lint` | `validation` |
| `4` | Resolution error — flow/reference/provider not found, bad `${{ }}`/input | `resolution` |
| `5` | Environment error — a provider method failed, or a preflight capability check failed | `environment` |
| `124` | Timeout (`--timeout` exceeded) | `timeout` |
| `130` | Interrupted (SIGINT) | `interrupt` |

This keeps the hexagonal rule: the core returns categories, the driving adapter maps them. An
HTTP-server host would map the *same* categories to status codes instead. The eval-as-CI-gate wedge
falls straight out: a missed `eval` threshold is `run_failure` → exit 1 → blocks the merge.

---

## Implementation layout

`tmx-cli/src/`: `main.rs` (parse → compose → use case → map error to exit), `args.rs` (clap),
`config.rs` (layering), `compose.rs` (the [composition root](02-crate-architecture.md#composition-root)),
`commands/` (one thin module per command). The exit-code mapping is a single `fn exit_code(&RunError)
-> i32` in `main.rs`.

---

## Assumptions and open questions

**Assumptions**

- `clap` (derive) expresses the hybrid primary/noun surface, the `--long`/`-short`/`--no-` boolean and
  `--flag value`/`--flag=value` conventions, and stacked POSIX shorts.
- A TTY check selects `pretty` vs `json` defaults; `NO_COLOR` and `--color`/`--no-color` are honoured.

**Decisions**

- *Exit-code mapping lives only in the CLI.* **`fn exit_code(&RunError)` in `main.rs`; the core never
  names a code.** Per [`RUNTIME.md` decision 5](../RUNTIME.md#design-decisions): non-CLI hosts map the
  same categories their own way.
- *stdout is the final state; progress is stderr.* **`tmx run | jq` works with no flag.** Per
  [`CLI.md` decision 3](../CLI.md#design-decisions); `--format pretty/ndjson` adjust the renderer.
- *No `-o` flag; redirect instead.* **`--format` selects the renderer, shell `>` writes anywhere,
  `--state-out` is the one explicit file convenience.** Per [`CLI.md` decision 6](../CLI.md#design-decisions),
  avoiding the `-o FILE`/`--output json` collision.
- *`--matrix` lowers to `map`; an authored `map` wins.* Per [`CLI.md` decision 8](../CLI.md#design-decisions):
  the authored definition is the source of truth; silent wrapping would surprise.

**Open questions**

- *`--watch` and the run loop.* `-w` re-runs on file change; how it interacts with the run store
  (one record per re-run, or a session?) and cancellation is unspecified.
- *Library/HTTP driving adapters.* The use cases support them, but only the CLI adapter is specified;
  the others are deferred (see [00-overview.md](00-overview.md#assumptions-and-open-questions)).
