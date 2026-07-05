# Done Certificate — Task 21: Filesystem adapter (`file`)

**Task:** [21-filesystem_adapter.md](21-filesystem_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

- **O2 — A missing-path read is a typed `RunError` and an over-cap read returns `output_too_large`, not a panic (negative space).**
  - *Claim:* a read of a missing path is a typed `RunError`; a read whose captured content exceeds `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large`; neither path panics.
  - *Evidence to collect:* run the negative-space tests — a missing-path read (expect a typed `RunError`, e.g. a not-found category) and an over-cap read (expect `output_too_large`); read `crates/tmx-adapters/src/fs.rs` for the `std::io::Error`→`RunError` translation and the `CAPTURED_OUTPUT_MAX_BYTES` bound.
  - *Checks:* confirm the read cap is `CAPTURED_OUTPUT_MAX_BYTES` (named constant) and every `std::io` error path maps to a typed `RunError` — no `unwrap`/`expect` on a filesystem call.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; run the adapter's tests with its Cargo feature enabled. Confirm the read cap is a named constant in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☐ unverified

- **O4 — Run a flow chaining write → read → move → exists and confirm the state reflects each step, then confirm the missing-path error (Reviewable).**
  - *Claim:* a reviewer can run a flow chaining write → read → move → exists and observe state reflecting each step (content read back, moved path exists, source gone), then run a missing-path read and observe the typed error.
  - *Evidence to collect:* run the reviewable chained flow; observe final state per step, then run a missing-path read and observe the typed `RunError`.
  - *Status:* ☐ unverified

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- The adapter is behind a Cargo feature: the validator must build with that feature enabled to exercise the tests, and confirm the default-feature build (and the Task 17 exec/assert path) still passes with the feature off.
- Confirm the `encoding` set (utf-8, base64, any binary variant) round-trips; copy/move across vs within a directory is out of the DoD scope.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
