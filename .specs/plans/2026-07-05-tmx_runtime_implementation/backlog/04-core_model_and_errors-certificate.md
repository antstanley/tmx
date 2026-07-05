# Done Certificate — Task 04: Core runtime model and error type

**Task:** [04-core_model_and_errors.md](04-core_model_and_errors.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

- **O2 — A serialized `TaskResult`, `Scorecard`, `Event`, and `RunRecord` each validate against the sidecar schema; an out-of-enum status is unrepresentable.**
  - *Claim:* representative `TaskResult`, `Scorecard`, `Event`, and `RunRecord` serializations validate against the sidecar schema, and an out-of-enum `RunStatus`/`TaskStatus`/`ErrorCategory` cannot be constructed.
  - *Evidence to collect:* run the serialize-and-validate test and confirm the four named types validate against [`.specs/canonical-types.schema.json`](../../../canonical-types.schema.json). Read the `RunStatus`/`TaskStatus`/`ErrorCategory` definitions and confirm they are closed Rust enums (no catch-all/`String` variant), so an out-of-enum value is unrepresentable; confirm a test asserts these are matched exhaustively.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-core` pulls in no async runtime or I/O crate, and no `anyhow` (the task uses `thiserror` only). Task 04 touches `canonical-types.schema.json` if runtime output shapes changed — run `scripts/validate.sh` and expect it clean.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the serialize-and-validate test to confirm the runtime types match the canonical schema.**
  - *Claim:* a reviewer can run the serialize-and-validate test and observe every runtime type's serialization validate against `canonical-types.schema.json`.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core` (the serialize-and-validate test) and observe every representative value validate against [`.specs/canonical-types.schema.json`](../../../canonical-types.schema.json) with zero validation errors.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress.

## Residue

- Task 04 depends on Task 02 (integer widths, limits) and Task 03 (input model) — confirm `ResolvedFlow`/`Pipeline` embed the Task 03 `Flow`/`Task` types rather than re-declaring them.
- If `canonical-types.schema.json` is edited to add a runtime `$def`, both `scripts/validate.sh` and the schema's own internal consistency must be checked, and O3's schema-change branch applies.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
