# TMX schema examples

Worked examples for the [`tmx.schema.json`](../tmx.schema.json) and
[`tmx-provider.schema.json`](../tmx-provider.schema.json) draft schemas. Every file
here is validated in CI / pre-push (see [Validating](#validating)).

## Combined Flow (one file, four formats)

The same Flow, authored in each supported source format and kept **semantically
identical** (parity is asserted by the validator):

| File | Format |
| --- | --- |
| [`single-file-flow.json`](./single-file-flow.json) | JSON (canonical) |
| [`single-file-flow.yaml`](./single-file-flow.yaml) | YAML |
| [`single-file-flow.toml`](./single-file-flow.toml) | TOML |
| [`single-file-flow.jsonc`](./single-file-flow.jsonc) | JSONC (JSON + comments) |
| [`minimal-flow.json`](./minimal-flow.json) | smallest valid Flow (`tasks` only) |

It exercises every built-in task type (`exec`, `run`, `fetch`, `file`, `store`,
`chat-completion`, `assert`, `flow`), inline environment + context, lifecycle hooks,
an `if` skip, and a per-task `secrets` declaration.

## Bounded fan-out, evaluations, and typed output

Worked examples for the newer task types and the `produces` contract. Each is provided in
**YAML, JSON and TOML**, kept semantically identical (parity is asserted by the validator, as
for `single-file-flow.*`):

| Example (`.yaml` / `.json` / `.toml`) | Demonstrates |
| --- | --- |
| [`map-fanout`](./map-fanout.yaml) | `map` — bounded fan-out of a per-item sub-flow over a collection, with `concurrency` and `continueOnError`; plus a `produces` contract |
| [`eval`](./eval.yaml) | `eval` — scoring an LLM `subject` over a `dataset` with matcher + `llmRubric` scorers and a `threshold`, then gating with `assert` |
| [`typed-output`](./typed-output.yaml) | `produces` — declaring a task's output JSON Schema so downstream `${{ tasks.NAME.field }}` references can be linted |

## Standalone artifacts (folder layout)

The README's "standalone files in a folder" style, with mixed formats inheriting in a
single folder — [`folder-layout/`](./folder-layout):

| File | `kind` |
| --- | --- |
| [`folder-layout/environment.toml`](./folder-layout/environment.toml) | environment |
| [`folder-layout/context.yaml`](./folder-layout/context.yaml) | context |
| [`folder-layout/task-1.jsonc`](./folder-layout/task-1.jsonc) | task |
| [`folder-layout/task-2.yaml`](./folder-layout/task-2.yaml) | task (context merge + precedence) |

[`standalone/`](./standalone) holds the same three artifact kinds as plain JSON.

## Environment provider manifest

[`provider-manifest.yaml`](./provider-manifest.yaml) — a `flow`-type provider that
implements the required `bootstrap` / `deploy` / `clean` / `destroy` methods, validated
against [`tmx-provider.schema.json`](../tmx-provider.schema.json).

## Validating

All schemas and examples are validated by one script:

```bash
scripts/validate.sh                       # everything (meta-schema, instances, parity)
scripts/validate.sh path/to/file.yaml     # just one file (dispatched by `kind`)
```

It uses [`uv`](https://docs.astral.sh/uv/) when available (no committed venv); otherwise
it creates `.venv-tmx/` on first run. Each instance is dispatched to the right schema by
its `kind` field.

### Pre-push enforcement

This repo uses **Jujutsu (jj)**, which does **not** run Git hooks. So:

- **Pushing with jj:** use [`scripts/push.sh`](../../scripts/push.sh) — it validates,
  then runs `jj git push`. (`jj git push` alone skips the Git hook.)
- **Plain `git push`** (colocated repo / CI): the `.githooks/pre-push` hook runs the
  same validation. Enable once with `git config core.hooksPath .githooks`.
- **CI** remains the authoritative gate — run `scripts/validate.sh`.
