# Changelog

All notable changes to the **TMX spec** are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the spec follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The version is carried in each
schema's `$id` path (e.g. `https://tmx.dev/schemas/0.2.0/tmx.schema.json`).

> TMX is an **early-stage spec with no runtime yet** — these entries describe changes to the
> data model (JSON Schema), docs, examples, and validation tooling, not a shipped binary.
> While the spec is pre-1.0, minor versions may include breaking changes.

## [Unreleased]

_Nothing yet._

## [0.2.0] - 2026-05-31

A competitiveness pass adding the task-model features that close the biggest gaps against
shipping tools (CI matrices, AWS Step Functions, LLM-eval harnesses) while preserving TMX's
declarative, sequential-by-default identity. Rationale: see
[`docs/SCHEMA.md`](./docs/SCHEMA.md) §"Competitiveness-pass additions" (decisions 13–15) and
[`docs/comparison.md`](./docs/comparison.md).

### Added

- **`map` task type** — bounded fan-out. Runs a single inner task (or `flow` import) once per
  element of `items`, binding each as `${{ <as>.* }}` (default `item`), with optional bounded
  `concurrency` and `continueOnError`, collecting an ordered array under the task name. The one
  deliberate exception to sequential-only execution (no general branching, DAG, or unbounded
  parallelism).
- **`eval` task type** — measurement. Scores a `subject`'s output against one or more `scorers`,
  optionally over a `dataset` of cases, emitting a scorecard (`cases` / `summary` / `passed`).
  Scorer kinds: `matcher` (reuses the `assert` Vitest matchers), `llmRubric` (model-graded), and
  `exec`/`run` (custom). Optional `threshold` policy gates the task. Distinct from `assert`
  (measure vs gate).
- **`produces` task field** — an optional JSON Schema (Draft 2020-12) declaring a task's output
  shape. Purely declarative; enables static linting of downstream `${{ tasks.NAME.field }}`
  references, autocomplete, and an optional runtime conformance check.
- **`matcherName` shared `$def`** — the Vitest matcher enum, extracted so both `assert` and the
  `eval` `matcher` scorer reference one vocabulary.
- **Examples** — `map-fanout`, `eval`, and `typed-output`, each in YAML/JSON/TOML (parity-checked).
  `provider-manifest.yaml` now exercises `map` (in `bootstrap`) and `produces` (in `deploy`).
- **`docs/comparison.md`** — a 52-tool landscape comparison and TMX positioning analysis.
- **Spec versioning** — both schemas now carry the version in their `$id` path, with a top-level
  `$comment`. (Resolves the versioning open question in `SCHEMA.md`.)

### Changed

- **Schema `$id`s are now versioned**: `https://tmx.dev/schemas/0.2.0/{tmx,tmx-provider}.schema.json`.
  The provider manifest's cross-`$ref`s to the task definition are pinned to the versioned `$id`.
- **Control-flow wording** in `README.md` and `tmx.schema.json` (`taskList`/`tasks`) now notes the
  bounded `map` exception instead of stating "no parallel execution".
- **`assert`** is documented explicitly as a boolean **gate**, pointing to `eval` for continuous
  **measurement**.
- **Provider method bodies** inherit the new task types automatically (they `$ref` the task
  definition) — documented and demonstrated; no provider-schema structural change required.
- **Validator** (`scripts/validate_examples.py`): cross-format parity is now enforced for *every*
  multi-format example group (keyed by directory + stem), not just `single-file-flow.*`.

### Notes

- No backward-incompatible changes to existing documents: `map`/`eval` are additive task types and
  `produces` is an optional field; existing 0.1.0 Flows remain valid.
- The `$id` change is a parser-facing identifier change only; example documents do not reference
  `$id` and are unaffected.

## [0.1.0] - 2026-05-30

Initial draft of the TMX data model as JSON Schema (Draft 2020-12), plus worked examples and
validation tooling. No runtime — design phase.

### Added

- **Core schema** ([`docs/tmx.schema.json`](./docs/tmx.schema.json)) — a **Flow** =
  optional `environment` → optional `context` → required `tasks`. Tasks run in sequence; the only
  control flow is a per-task `if` skip. Each task's output is merged into the Pipeline state under
  its `name` (override with `output`).
- **Built-in task types** — `exec`, `run`, `fetch`, `file`, `store`, `chat-completion`, `assert`,
  and `flow` (import a Flow as a user-defined task), selected via a `type`-discriminated `with`.
- **`assert`** uses [Vitest `expect`](https://vitest.dev/api/expect.html) matchers (`not: true`
  for negation).
- **Task collections** may be an ordered **array** or a name-keyed **map**; in map form a string
  value is shorthand for an `exec` task.
- **Context** — `env`, `secrets` (auto-masked everywhere; opt-in per task via `secrets`), and
  lifecycle `hooks` (`create`/`change`/`destroy`/`error`).
- **Environment** — declarative, open object (`os`/`arch`/`platform`/`provider`/`runtime`/`image`/
  `resources`/`bootstrap`/`options`) materialised by a pluggable provider.
- **Flow `inputs`** — typed, declarable input variables supplied via `--input k=v` or a calling
  `flow` task, read as `${{ inputs.NAME }}`; `${{ ... }}` interpolation for secrets/state/inputs.
- **Provider manifest schema** ([`docs/tmx-provider.schema.json`](./docs/tmx-provider.schema.json))
  — `binary` | `flow` providers implementing `bootstrap`/`deploy`/`clean`/`destroy`.
- **Multi-format authoring** — YAML / JSON / JSONC / TOML all parse to one model; an optional
  `kind` discriminator (`flow`/`environment`/`context`/`task`/`provider`) lets a loader dispatch
  by content rather than filename.
- **Examples** in all four formats (kept semantically identical) and the
  [`scripts/validate.sh`](./scripts/validate.sh) validator (meta-schema check, `kind` dispatch,
  cross-format parity), with `jj`/`git` pre-push enforcement.

[Unreleased]: https://github.com/antstanley/tmx/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/antstanley/tmx/releases/tag/v0.2.0
[0.1.0]: https://github.com/antstanley/tmx/releases/tag/v0.1.0
