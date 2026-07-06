# Task 22 — Object store adapter (`store`)

**Plan:** [plan.md](../plan.md) · **Certificate:** [22-object_store_adapter-certificate.md](22-object_store_adapter-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Executor ports (`ObjectStore`), §Adding a backend
**Depends on:** 05, 17
**Produces:** `S3ObjectStore` — the `store` executor covering get/put/delete/list/head against an S3-compatible endpoint
**Pointers:** `crates/tmx-adapters/src/store.rs` (new), `crates/tmx-cli/src/compose.rs` (wire into the bundle)

## Steps

- [x] Implement the `ObjectStore` port over `store` operations: get, put, delete, list, head, taking `endpoint`/`region`/`credentials`.
- [x] Bound a fetched object's captured size by `CAPTURED_OUTPUT_MAX_BYTES` and translate every S3-SDK error into a typed `RunError`; keep the credential values maskable.
- [x] Wire the adapter into the composition root, behind its Cargo feature.
- [x] Add integration tests against a local S3-compatible endpoint (marked `#[ignore]` where a real backend is needed) plus typed-error tests for a missing key.

## Definition of done

- [x] `store` get/put/delete/list/head operate against an S3-compatible endpoint, and a `get` of a missing key is a typed `RunError`.
- [x] Credentials never appear in an emitted payload (they route through the Masker), and an oversized object returns `output_too_large` (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run a flow doing put → head → get → list → delete against a local endpoint and confirm the state and the masked credentials.
