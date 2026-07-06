# Done Certificate — Task 21: Filesystem adapter (`file`)

**Task:** [21-filesystem_adapter.md](21-filesystem_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — discharged by an independent verifier

> This certificate is a verification protocol for Task 21. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 21) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `LocalFileSystem` is the `file` executor covering read/write/append/delete/copy/move/exists with `encoding`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Wiring `LocalFileSystem` into the Task 17 composition root (`crates/tmx-cli/src/compose.rs`), behind its Cargo feature, must not change the existing `exec`/`assert` run path.

## Obligations

- **O1 — Each `file` operation executes against the local filesystem with the requested encoding; `exists` reports correctly for present and absent paths.**
  - *Claim:* read/write/append/delete/copy/move/exists execute against the local filesystem honouring the `encoding` field; `exists` returns true for a present path and false for an absent one.
  - *Evidence to collect:* read `crates/tmx-adapters/src/fs.rs`; run the per-operation tests (each of read/write/append/delete/copy/move/exists) against a temp dir or the `MemFileSystem` fake in `crates/tmx-testkit`; confirm `encoding` is honoured (e.g. utf-8 vs base64) and `exists` returns correctly for both present and absent paths.
  - *Checks:* trace that the `file` task routes through the `FileSystem` port to `LocalFileSystem` via the `TaskDispatcher`.
  - *Status:* ☑ SATISFIED — `crates/tmx-adapters/src/fs.rs` read; all 7 per-op tests pass under `cargo nextest run -p tmx-adapters fs::` (14/14): write/read round-trip (utf-8), append (creates + extends), delete, copy (source kept), move (source gone), exists (present=true, absent=false). Encoding honoured: utf-8/binary pass-through, base64 pinned to RFC 4648 §10 vectors (`base64_matches_rfc4648_vectors`), non-UTF-8 bytes carried via base64 (`read_base64_encodes_the_raw_bytes`). Route traced: `preflight.rs:830` requires `Capability::File` (now advertised by `compose.rs:126`) → `dispatch.rs:91-95` `TaskWith::File` → `build_file_op` → `ports.file.op` → `LocalFileSystem`; confirmed end-to-end by running a real `file` flow through the built `tmx` binary (write→read→move→exists all ok, on-disk state matched).

- **O2 — A missing-path read is a typed `RunError` and an over-cap read returns `output_too_large`, not a panic (negative space).**
  - *Claim:* a read of a missing path is a typed `RunError`; a read whose captured content exceeds `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large`; neither path panics.
  - *Evidence to collect:* run the negative-space tests — a missing-path read (expect a typed `RunError`, e.g. a not-found category) and an over-cap read (expect `output_too_large`); read `crates/tmx-adapters/src/fs.rs` for the `std::io::Error`→`RunError` translation and the `CAPTURED_OUTPUT_MAX_BYTES` bound.
  - *Checks:* confirm the read cap is `CAPTURED_OUTPUT_MAX_BYTES` (named constant) and every `std::io` error path maps to a typed `RunError` — no `unwrap`/`expect` on a filesystem call.
  - *Status:* ☑ SATISFIED — `missing_path_read_is_a_typed_not_found_error` passes (category `RunFailure`, code `file_not_found`); `over_cap_read_is_output_too_large` passes (code `output_too_large`); `at_cap_read_is_allowed` pins the boundary and a mutation check (cap guard `>`→`>=`, fs.rs:136) tripped it, then was reverted (re-run green). Default cap is `tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES` (fs.rs:103); `read_capped` pulls at most cap+1 bytes via `take(saturating_add(1))` so an over-cap file is never fully buffered. Every `std::io::Error` maps through `FsError::io` → `From<FsError> for RunError` (NotFound → `file_not_found`, else `file_io_failed`, both with path context); zero `unwrap`/`expect` in non-test code (grep confirmed). Missing-path error also observed through the CLI: exit 1, `task: error readmissing: file not found: <path>`, no panic.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; run the adapter's tests with its Cargo feature enabled. Confirm the read cap is a named constant in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☑ SATISFIED — verifier ran independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 264/264 passed (14 are the new fs tests, run with the `fs` feature on by default); `cargo check -p tmx-adapters --no-default-features` compiles (feature-off build intact); `bash scripts/purity.sh` green. Read cap is the named `CAPTURED_OUTPUT_MAX_BYTES`; the only new consts are encoding tokens and the RFC 4648 base64 alphabet/group sizes — identifiers and protocol constants, not unnamed numeric bounds. No schema/docs change → `scripts/validate.sh` not required.

- **O4 — Run a flow chaining write → read → move → exists and confirm the state reflects each step, then confirm the missing-path error (Reviewable).**
  - *Claim:* a reviewer can run a flow chaining write → read → move → exists and observe state reflecting each step (content read back, moved path exists, source gone), then run a missing-path read and observe the typed error.
  - *Evidence to collect:* run the reviewable chained flow; observe final state per step, then run a missing-path read and observe the typed `RunError`.
  - *Status:* ☑ SATISFIED — exercised twice: (1) unit test `chained_write_read_move_exists_reflects_each_step` passes; (2) verifier built `tmx` and ran a real 5-task `file` flow — write ok, read returned the written content, move ok, `exists(src)`=false, `exists(dst)`=true, exit 0, and the on-disk file held the payload at the destination. Then a missing-path read flow: typed `file not found: <path>` error, exit 1 (run_failure), no panic.

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☑ PRESERVED — full `cargo nextest run` 264/264 including the Task 17 exec/assert integration tests; the unwired-capability exit-5 test was correctly retargeted from `file` (now real) to `store` (still a denying stub) and passes; the composition test now asserts `File` real and `Store`/`Chat` absent.

## Residue

- The adapter is behind a Cargo feature: the validator must build with that feature enabled to exercise the tests, and confirm the default-feature build (and the Task 17 exec/assert path) still passes with the feature off.
- Confirm the `encoding` set (utf-8, base64, any binary variant) round-trips; copy/move across vs within a directory is out of the DoD scope.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: O1–O4 all SATISFIED with independently produced evidence — per-op + encoding tests green, negative space (missing path, over-cap, unknown encoding) typed and mutation-checked at the cap boundary, repo gates (fmt/clippy/nextest 264/264/purity/no-default-features check) clean, and the reviewable write→read→move→exists chain plus missing-path error exercised through the real CLI binary with observed on-disk state; the Task 17 exec/assert path is preserved.
