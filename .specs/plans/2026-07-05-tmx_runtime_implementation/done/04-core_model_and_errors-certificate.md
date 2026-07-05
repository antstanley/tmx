# Done Certificate — Task 04: Core runtime model and error type

**Task:** [04-core_model_and_errors.md](04-core_model_and_errors.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

> This certificate is a verification protocol for Task 04. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 04) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The `tmx-core` runtime entities and the typed `RunError`/`ErrorCategory`, serializing to shapes that validate against `canonical-types.schema.json`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** None — greenfield foundation; no prior behavior to preserve.

## Obligations

- **O1 — Every runtime `$def` in `canonical-types.schema.json` has a type whose serialization validates against it, and `RunError` carries category/code/message/task/path.**
  - *Claim:* each runtime `$def` has a Rust type whose serialization validates against the sidecar schema, and `RunError` carries all five named fields.
  - *Evidence to collect:* read the planned `crates/tmx-core/src/model.rs` and `crates/tmx-core/src/error.rs`; confirm one type per runtime `$def` in [`.specs/canonical-types.schema.json`](../../../canonical-types.schema.json) (`RunId`, `Milliseconds`, `Timestamp`, `RunStatus`, `TaskStatus`, `PipelineState`, `MessageWrapper`, `BlobWrapper`, `ErrorCategory`, `RunError`, `Diagnostic`, `TaskResult`, `EvalCase`, `EvalSummary`, `Scorecard`, `Event`, `RunRecord`). Confirm `RunError` has `category`/`code`/`message`/`task`/`path` and `ErrorCategory ∈ {RunFailure, Validation, Resolution, Environment, Timeout, Interrupt}`. Run the serialize-and-validate test in `crates/tmx-core` that serializes representative values and validates each against the sidecar schema; expect all pass. Confirm `RunId` is a UUIDv7 newtype rendered lowercase-hyphenated, `PipelineState` is a `serde_json::Value` invariant-checked as an object, and fixed-width integers cross the serialization boundary.
  - *Status:* ☑ SATISFIED — all 17 runtime `$def`s have a Rust type in `model.rs`/`error.rs`; cross-checked field-by-field against the schema (camelCase renames `startedAt`/`weightedMean`/`passRate`/`finishedAt`/`finalState` correct; `additionalProperties:false` honoured; required fields always emitted, optionals `skip_serializing_if`). `RunError` (error.rs:82-97) carries `category`/`code`/`message`/`task`/`path`; `ErrorCategory` (error.rs:25-40) is exactly the six-category closed enum. `RunId` (model.rs:85-122) is a `#[serde(transparent)]` newtype whose `validate_run_id` mirrors the schema pattern index-by-index (hyphens 8/13/18/23, version `7` at 14, variant `[89ab]` at 19, lowercase hex elsewhere). `PipelineState` (model.rs:250-309) is an object-checked `serde_json::Value`. Durations are `Milliseconds(u64)`, counts/indices `u32` — no `usize` crosses the boundary. `every_runtime_def_has_a_type_that_validates` (10/10 tmx-core tests, run observed) validates a representative value for every type against the real `jsonschema` validator with zero errors.

- **O2 — A serialized `TaskResult`, `Scorecard`, `Event`, and `RunRecord` each validate against the sidecar schema; an out-of-enum status is unrepresentable.**
  - *Claim:* representative `TaskResult`, `Scorecard`, `Event`, and `RunRecord` serializations validate against the sidecar schema, and an out-of-enum `RunStatus`/`TaskStatus`/`ErrorCategory` cannot be constructed.
  - *Evidence to collect:* run the serialize-and-validate test and confirm the four named types validate against [`.specs/canonical-types.schema.json`](../../../canonical-types.schema.json). Read the `RunStatus`/`TaskStatus`/`ErrorCategory` definitions and confirm they are closed Rust enums (no catch-all/`String` variant), so an out-of-enum value is unrepresentable; confirm a test asserts these are matched exhaustively.
  - *Status:* ☑ SATISFIED — `TaskResult`, `Scorecard`, `Event` (all 11 variants), and `RunRecord` (minimal + full) validate in `every_runtime_def_has_a_type_that_validates`. `RunStatus`/`TaskStatus`/`ErrorCategory` (and `Severity`) are closed enums with no `String`/catch-all variant (out-of-enum unrepresentable at the type level); exhaustiveness is pinned by wildcard-free `as_str` matches plus `run_status_and_task_status_tokens_match_serialisation_exhaustively` and `category_wire_tokens_are_snake_case_and_match_serialisation`. Negative space independently verified: `the_validator_rejects_out_of_contract_shapes` rejects an out-of-enum status, an extra property, a missing required field, a bad `RunId`, and a negative `Milliseconds`. I actively injected a serialization drift (flipped `TaskResult` to `snake_case`) and observed the schema test trip — `"startedAt" is a required property` + `Additional properties are not allowed ('started_at')` — then reverted; the green result is therefore non-vacuous.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-core` pulls in no async runtime or I/O crate, and no `anyhow` (the task uses `thiserror` only). Task 04 touches `canonical-types.schema.json` if runtime output shapes changed — run `scripts/validate.sh` and expect it clean.
  - *Status:* ☑ SATISFIED — I ran (observed, not claimed): `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo build` clean; `cargo nextest run` = 23 tests, 23 passed, 0 skipped; `bash scripts/purity.sh` green (`tmx-schema, tmx-core, tmx-testkit carry no I/O or async dependency edge`). `Cargo.toml` deps are `tmx-schema` + pure data crates (`serde`, `serde_json`, `indexmap`, `thiserror`); `anyhow` absent; `jsonschema` is dev-only, so invisible to the `--edges normal` gate. UUIDv7 layout bounds are named units-last consts in `model.rs` (format structure, not tunable engine limits, which stay in `tmx-schema::limits`). `canonical-types.schema.json` was NOT modified by this task (0 hits in the diff), so O3's schema-change branch and `scripts/validate.sh` do not apply.

- **O4 — Reviewable: run the serialize-and-validate test to confirm the runtime types match the canonical schema.**
  - *Claim:* a reviewer can run the serialize-and-validate test and observe every runtime type's serialization validate against `canonical-types.schema.json`.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core` (the serialize-and-validate test) and observe every representative value validate against [`.specs/canonical-types.schema.json`](../../../canonical-types.schema.json) with zero validation errors.
  - *Status:* ☑ SATISFIED — ran `cargo nextest run -p tmx-core`: 10 tests, 10 passed, 0 skipped, including `every_runtime_def_has_a_type_that_validates` and `the_validator_rejects_out_of_contract_shapes`. Every representative runtime value validates against the sidecar schema with zero validation errors; the reviewable evidence is reproducible from the repo root.

## Regression check

- No existing callers in scope — greenfield; nothing to regress.

## Residue

- Task 04 depends on Task 02 (integer widths, limits) and Task 03 (input model) — confirm `ResolvedFlow`/`Pipeline` embed the Task 03 `Flow`/`Task` types rather than re-declaring them.
- If `canonical-types.schema.json` is edited to add a runtime `$def`, both `scripts/validate.sh` and the schema's own internal consistency must be checked, and O3's schema-change branch applies.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED. Every one of the 17 runtime `$def`s has a Rust type whose real `Serialize` output validates against `canonical-types.schema.json` via a real JSON-Schema validator (`every_runtime_def_has_a_type_that_validates`, 10/10 tmx-core tests). `RunError` carries category/code/message/task/path; `RunStatus`/`TaskStatus`/`ErrorCategory`/`Severity` are closed enums (out-of-enum unrepresentable, exhaustiveness pinned by wildcard-free `as_str`). Negative space independently exercised — an injected `snake_case` drift tripped the schema test on `startedAt` and was reverted, so the green suite is non-vacuous. All four cargo gates + `scripts/purity.sh` observed green from the repo root; no `anyhow`, `jsonschema` dev-only; the sidecar schema was not modified. Greenfield — no regression surface.
