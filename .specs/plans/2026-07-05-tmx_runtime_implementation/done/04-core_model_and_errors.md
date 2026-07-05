# Task 04 — Core runtime model and error type

**Plan:** [plan.md](../plan.md) · **Certificate:** [04-core_model_and_errors-certificate.md](04-core_model_and_errors-certificate.md)

**Implements:** [01-domain-model.md](../../../01-domain-model.md) §Runtime entities, §ID scheme, §Pipeline lifecycle; [08-errors-and-observability.md](../../../08-errors-and-observability.md) §Error model; the runtime contract [canonical-types.schema.json](../../../canonical-types.schema.json)
**Depends on:** 02, 03
**Produces:** the `tmx-core` runtime entities and the typed `RunError`/`ErrorCategory`, serializing to shapes that validate against `canonical-types.schema.json`
**Pointers:** `crates/tmx-core/src/model.rs` (new), `crates/tmx-core/src/error.rs` (new), `crates/tmx-core/src/lib.rs`, `.specs/canonical-types.schema.json`

## Steps

- [x] Define the runtime entities: `RunId` (UUIDv7 newtype, lowercase-hyphenated), `ResolvedFlow`, `Pipeline`, `PipelineState` (a `serde_json::Value` invariant-checked as an object), `Scope`, `TaskResult`, `Scorecard`/`EvalCase`/`EvalSummary`, `Diagnostic`, the tagged `Event` enum, `RunRecord`, and the `RunStatus`/`TaskStatus` enums.
- [x] Define `RunError { category: ErrorCategory, code: &'static str, message: String, task: Option<String>, path: Option<String> }` and `ErrorCategory { RunFailure, Validation, Resolution, Environment, Timeout, Interrupt }` using `thiserror`; do not pull in `anyhow`.
- [x] Derive `Serialize` on the emitted types (and `Deserialize` where seeded from disk, e.g. `--state-in`), using fixed-width integers across the serialization boundary.
- [x] Add a test that serializes representative values and validates them against `canonical-types.schema.json`, and that asserts `RunStatus`/`TaskStatus`/`ErrorCategory` are matched exhaustively.

## Definition of done

- [x] Every runtime `$def` in [canonical-types.schema.json](../../../canonical-types.schema.json) has a type whose serialization validates against it, and `RunError` carries category/code/message/task/path.
- [x] A serialized `TaskResult`, `Scorecard`, `Event`, and `RunRecord` each validate against the sidecar schema; an out-of-enum status is unrepresentable (negative space, enforced by the type).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the serialize-and-validate test to confirm the runtime types match the canonical schema.
