# Done Certificate — Task 22: Object store adapter (`store`)

**Task:** [22-object_store_adapter.md](22-object_store_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — discharged by an independent verifier (real S3-compatible endpoint exercised: LocalStack on localhost:4566)

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
  - *Status:* ☑ SATISFIED — read `crates/tmx-adapters/src/store.rs`; a live LocalStack S3 (localhost:4566) was available, so the `#[ignore]` integration tests were RUN for real: `put_head_get_list_delete_round_trip` and `get_of_a_missing_key_is_object_not_found` both PASS (`cargo nextest run -p tmx-adapters --features store --run-ignored only store::` → 2 passed). The missing-key translation unit test `missing_key_get_is_a_typed_object_not_found` passes (typed `object_not_found` RunFailure). SigV4 is pinned deterministically to the published AWS `get-vanilla` vector (`sigv4_matches_the_aws_get_vanilla_vector`). Routing traced: `dispatch.rs:97-99` builds a `StoreOp` and calls `ports.store.op(op)` → the composed `S3ObjectStore` (`compose.rs`, `store` feature on).

- **O2 — Credentials never appear in an emitted payload (they route through the Masker); an oversized object returns `output_too_large` (negative space).**
  - *Claim:* credential values never appear in an emitted payload — they route through the Masker; an object over `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large`.
  - *Evidence to collect:* run the masking test asserting an emitted `store` payload/state carries no raw credential value; run the over-cap `get` (expect `output_too_large`); read `crates/tmx-adapters/src/store.rs` for the S3-SDK-error→`RunError` translation and that the credential values stay maskable.
  - *Checks:* trace that each credential value is registered with the Masker before any output, so no emitted payload carries a raw credential; confirm the captured-size bound is `CAPTURED_OUTPUT_MAX_BYTES`.
  - *Status:* ☑ SATISFIED — masking test `credentials_are_surfaced_for_masking_and_redacted` is genuine (registers surfaced values with the real tmx-core `Masker`, asserts the raw secret does not survive `redact_value`), plus `store_results_never_carry_a_credential_value` (structural: `StoreResult` variants carry object data only) and `empty_credentials_are_not_registered_as_secrets` (negative space). Verified live: ran the CLI (built with `--features store`) against LocalStack with distinctive credentials (`verifierAKID123`/`verifiersecret456`) — grep over the emitted state and stderr of the round-trip flow, the missing-key flow, and the over-cap flow found zero occurrences of either value. Over-cap verified for REAL, not just at the translation boundary: put a 65 MiB object, `get` via the CLI → `task: error get-big: the object exceeds the captured-output cap of 67108864 bytes` (typed `output_too_large`; cap is the named `CAPTURED_OUTPUT_MAX_BYTES` = 64 MiB, `read_capped` reads in bounded chunks, store.rs:367-379). Note (non-gating, per Residue): the end-to-end Masker *registration* of adapter credentials is Task 24's produced wiring — `credential_values()` has no caller yet; the composition root must register it when Task 24 lands. Credentials provably never appear in an emitted payload today (structural + live grep).

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; run the adapter's tests with its Cargo feature enabled, and the `#[ignore]` integration tests where a local endpoint is available. Confirm the captured-size cap is a named constant in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☑ SATISFIED — independently run by the verifier: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` (default features) 264/264 passed; `cargo nextest run -p tmx-adapters --features store store::` 10/10 passed; `cargo nextest run -p tmx-adapters -p tmx-cli --features tmx-cli/store` 113/113 passed; the two `#[ignore]` integration tests run against a live endpoint, 2/2 passed; `scripts/purity.sh` green. The cap is `tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES`; new constants (`SECONDS_PER_DAY`/`_HOUR`/`_MINUTE`, `EMPTY_PAYLOAD_SHA256`, SigV4 tokens) are unit conversions and protocol tokens, not new bounds. No schema/example changed.

- **O4 — Run a flow doing put → head → get → list → delete against a local endpoint and confirm the state and the masked credentials (Reviewable).**
  - *Claim:* a reviewer can run a flow doing put → head → get → list → delete against a local S3-compatible endpoint and observe state reflecting each step (head metadata, got object, list includes the key, delete removes it) with credentials masked in the emitted state.
  - *Evidence to collect:* run the reviewable flow against a local endpoint (or the `MemObjectStore` fake where the real backend is unavailable — note the `#[ignore]` gating); observe the per-step state and the masked credentials.
  - *Status:* ☑ SATISFIED — RUN for real against LocalStack S3: a six-task flow (put → head → get → list → delete → head) through the `tmx run` CLI built with `--features store` exited 0 with state `{"put":{"ok":true},"head":{"exists":true,"sizeBytes":23},"get":…round-tripped bytes…,"list":["verify/roundtrip.txt"],"delete":{"ok":true},"head-after-delete":{"exists":false,"sizeBytes":null}}` — every step's state reflects the operation, the listing includes the key, and the object is gone after delete. No credential value appears anywhere in stdout/stderr (grep over distinctive injected credentials: zero hits). Feature-gate negative space also exercised: the default (no-feature) build running the same store flow exits 5 with `Environment [missing_capability]: no ObjectStore adapter is wired for a store task`.

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☑ PRESERVED — full default-feature suite 264/264 (includes the Task 17 compose test and all exec/assert CLI runs); the compose test is feature-aware and passes in BOTH configurations (default: `store` capability absent, denying stub; `--features tmx-cli/store`: capability advertised, 113/113). The default build keeps `DenyingObjectStore` (P3 holds). The `a_flow_needing_an_unwired_port_exits_five` test's switch from `store` to `chat-completion` is sound: `chat` is the port that stays unwired in every feature configuration, and the exit-5 behaviour for an unwired `store` was re-verified directly on the default binary.

## Residue

- The integration tests need a local S3-compatible endpoint (MinIO/LocalStack); where absent they are `#[ignore]` and the deterministic coverage is the `MemObjectStore` fake plus the missing-key typed-error test. The validator should record whether a real endpoint was exercised.
- The adapter is behind a Cargo feature — build with it enabled to exercise the tests.
- Credential masking depends on the Task 09 Masker and the Task 24 secret-registration path being present; confirm the masking assertion is genuine, not a no-op.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with a real S3-compatible endpoint exercised (LocalStack on localhost:4566, so no environment gap remained): the `#[ignore]` integration tests pass, the reviewable put→head→get→list→delete flow ran end-to-end through the `tmx` CLI with correct per-step state and zero credential leakage, a live 65 MiB over-cap `get` returned typed `output_too_large`, a live missing-key `get` returned typed `object_not_found`, SigV4 is pinned to the AWS `get-vanilla` vector, all gates (fmt/clippy/nextest in three feature configurations/purity) are clean, and the exec/assert path plus the default-build denying stub are preserved. Residue recorded: `credential_values()` awaits its Task 24 Masker-registration caller at the composition root; per-task `bucket` from `StoreWith` is validated by schema but the port does not thread it (adapter uses the composed bucket) — a known port-signature limitation, not a Task 22 defect.
