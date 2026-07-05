# Done Certificate — Task 22: Object store adapter (`store`)

**Task:** [22-object_store_adapter.md](22-object_store_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 22. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 22) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `S3ObjectStore` is the `store` executor covering get/put/delete/list/head against an S3-compatible endpoint, taking `endpoint`/`region`/`credentials`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Wiring `S3ObjectStore` into the Task 17 composition root (`crates/tmx-cli/src/compose.rs`), behind its Cargo feature, must not change the existing `exec`/`assert` run path.

## Obligations

- **O1 — `store` get/put/delete/list/head operate against an S3-compatible endpoint; a `get` of a missing key is a typed `RunError`.**
  - *Claim:* get/put/delete/list/head operate against an S3-compatible endpoint taking `endpoint`/`region`/`credentials`; a `get` of a missing key is a typed `RunError`.
  - *Evidence to collect:* read `crates/tmx-adapters/src/store.rs`; run the integration tests against a local S3-compatible endpoint (e.g. MinIO) — these may be `#[ignore]` where a real backend is needed, so run them explicitly, or exercise the deterministic path via the `MemObjectStore` fake in `crates/tmx-testkit`; run the typed-error test for a missing-key `get` (expect a typed `RunError`).
  - *Checks:* trace that the `store` task routes through the `ObjectStore` port to `S3ObjectStore` via the `TaskDispatcher`.
  - *Status:* ☐ unverified

- **O2 — Credentials never appear in an emitted payload (they route through the Masker); an oversized object returns `output_too_large` (negative space).**
  - *Claim:* credential values never appear in an emitted payload — they route through the Masker; an object over `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large`.
  - *Evidence to collect:* run the masking test asserting an emitted `store` payload/state carries no raw credential value; run the over-cap `get` (expect `output_too_large`); read `crates/tmx-adapters/src/store.rs` for the S3-SDK-error→`RunError` translation and that the credential values stay maskable.
  - *Checks:* trace that each credential value is registered with the Masker before any output, so no emitted payload carries a raw credential; confirm the captured-size bound is `CAPTURED_OUTPUT_MAX_BYTES`.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; run the adapter's tests with its Cargo feature enabled, and the `#[ignore]` integration tests where a local endpoint is available. Confirm the captured-size cap is a named constant in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☐ unverified

- **O4 — Run a flow doing put → head → get → list → delete against a local endpoint and confirm the state and the masked credentials (Reviewable).**
  - *Claim:* a reviewer can run a flow doing put → head → get → list → delete against a local S3-compatible endpoint and observe state reflecting each step (head metadata, got object, list includes the key, delete removes it) with credentials masked in the emitted state.
  - *Evidence to collect:* run the reviewable flow against a local endpoint (or the `MemObjectStore` fake where the real backend is unavailable — note the `#[ignore]` gating); observe the per-step state and the masked credentials.
  - *Status:* ☐ unverified

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- The integration tests need a local S3-compatible endpoint (MinIO/LocalStack); where absent they are `#[ignore]` and the deterministic coverage is the `MemObjectStore` fake plus the missing-key typed-error test. The validator should record whether a real endpoint was exercised.
- The adapter is behind a Cargo feature — build with it enabled to exercise the tests.
- Credential masking depends on the Task 09 Masker and the Task 24 secret-registration path being present; confirm the masking assertion is genuine, not a no-op.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
