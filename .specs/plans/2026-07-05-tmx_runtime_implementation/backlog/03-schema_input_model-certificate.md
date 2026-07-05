# Done Certificate — Task 03: Schema input model (the static Flow)

**Task:** [03-schema_input_model.md](03-schema_input_model.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The deserialize-only Rust mirror of every `tmx.schema.json` `$def`, so the whole example corpus loads into typed values with source order preserved.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** None — greenfield foundation; no prior behavior to preserve.

## Obligations

- **O1 — Every `$def` in `tmx.schema.json` has a corresponding type, and every example in `docs/examples/` deserializes without loss.**
  - *Claim:* each `$def` in `docs/tmx.schema.json` maps to a Rust type across `flow.rs`/`task.rs`/`context.rs`/`environment.rs`, and every artifact under `docs/examples/` deserializes without loss.
  - *Evidence to collect:* read the planned `crates/tmx-schema/src/{flow,task,context,environment}.rs`; enumerate the `$defs` in [`docs/tmx.schema.json`](../../../../docs/tmx.schema.json) and confirm each has a corresponding type (`Flow`, `InputSpec`, `Tasks`, `TaskEntry`, `Task`, `TaskWith` and its ten variants, `Context`, `Hook`, `SecretSource`, `Environment`, `Duration`). Run the corpus round-trip test in `crates/tmx-schema` that deserializes every file under `docs/examples/` (`single-file-flow.{yaml,json,jsonc,toml}`, `map-tasks.yaml`, `shorthand-tasks.json`, `eval.*`, `map-fanout.*`, `typed-output.*`, `minimal-flow.json`, `folder-layout/`, `standalone/`) and expect all to load.
  - *Status:* ☐ unverified

- **O2 — A map-form and an array-form Flow both round-trip in source order, and a malformed `with`/`type` pairing fails to deserialize.**
  - *Claim:* array-form and map-form task lists preserve document order through a round-trip, and a `with`/`type` mismatch is rejected at deserialize.
  - *Evidence to collect:* run the corpus round-trip test's order-preservation assertions on an array-form Flow and a map-form Flow (e.g. `map-tasks.yaml`) and confirm tasks emerge in source order. Add or confirm a negative test that a `Task` whose `with` payload does not match its `type` discriminant fails to deserialize.
  - *Checks:* confirm the map form (`Tasks::Map` / `TaskEntry`) deserializes into `indexmap::IndexMap`, not `std::collections::HashMap` or `serde_json::Map` — either of which would silently lose source key order.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-schema` pulls in no async runtime or I/O crate. Task 03 changes no schema or example (the corpus is consumed read-only), so `scripts/validate.sh` is not required by this task.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the corpus round-trip test and diff a re-serialized map-form Flow to confirm key order is preserved.**
  - *Claim:* a reviewer can run the corpus round-trip test and diff a re-serialized map-form Flow against its source to observe key order preserved.
  - *Evidence to collect:* run `cargo nextest run -p tmx-schema`; then re-serialize a map-form example (e.g. `map-tasks.yaml`) and diff the task key order against the source, observing identical order.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress.

## Residue

- The crate is deserialize-only; the O4 "re-serialize and diff" may require a test-only `Serialize` derive or a structural key-order assertion in place of a full round-trip re-emit — either satisfies the intent (key order preserved), a `Deserialize`-only regression does not.
- `folder-layout/` and `standalone/` are multi-file fixtures — confirm the round-trip test resolves the intended entry artifact rather than skipping the directory.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
