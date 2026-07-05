# Done Certificate — Task 02: Schema limits and the matcher vocabulary

**Task:** [02-schema_limits_and_matchers.md](02-schema_limits_and_matchers.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 02. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 02) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The single source-of-truth `tmx-schema::limits` module holds every named units-last constant, plus the closed `MatcherName` enum, each backed by compile-time sanity assertions.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** None — greenfield foundation; no prior behavior to preserve.

## Obligations

- **O1 — Every constant in the limits table exists with the exact units-last name and its documented default, referenced nowhere as a magic number.**
  - *Claim:* `crates/tmx-schema/src/limits.rs` defines all thirteen named units-last constants from the limits table with their documented defaults, and no default appears inline elsewhere.
  - *Evidence to collect:* read the planned `crates/tmx-schema/src/limits.rs` and check each of `STATE_SIZE_MAX_BYTES`, `FLOW_DEPTH_MAX`, `TASKS_PER_FLOW_MAX`, `FANOUT_WIDTH_MAX`, `CONCURRENCY_MAX`, `EXPR_LEN_MAX_BYTES`, `EXPR_DEPTH_MAX`, `JSON_DEPTH_MAX`, `CAPTURED_OUTPUT_MAX_BYTES`, `HOOK_TASKS_MAX`, `EVENT_LOG_MAX_BYTES`, `CANCEL_GRACE_MS`, `MASK_SCAN_LEN_MIN_BYTES` against the limits table in [`.specs/04-execution-engine.md#limits`](../../../04-execution-engine.md#limits) — name, unit suffix, and default value must match (e.g. `STATE_SIZE_MAX_BYTES` = 512 MiB, `FLOW_DEPTH_MAX` = 8, `EXPR_LEN_MAX_BYTES` = 4096). Confirm each carries a doc comment stating what it bounds and where enforced, and that config-tunable vs structurally-fixed constants are marked. Grep the workspace for the literal default values to confirm they are referenced by name, not inlined. Confirm explicit fixed-width integer types (no `usize` across a serialization boundary).
  - *Status:* ☐ unverified

- **O2 — `MatcherName` round-trips every schema spelling, and a unit test pins the variant count to the closed vocabulary size.**
  - *Claim:* `MatcherName` serde round-trips every `matcherName` spelling in `tmx.schema.json`, and a test asserts the variant count equals the closed vocabulary size (25) so adding or dropping a matcher fails.
  - *Evidence to collect:* read the planned `crates/tmx-schema/src/matcher.rs`; confirm `MatcherName` is a closed enum deriving serde with the schema's string spellings from the `matcherName` `$def` in [`docs/tmx.schema.json`](../../../../docs/tmx.schema.json) (25 value matchers, mock/promise excluded). Run the matcher round-trip unit test in `crates/tmx-schema` and expect every spelling to deserialize and re-serialize identically and the count assertion to hold. Confirm the count assertion is against a literal expected size so that adding or removing a variant fails the build or the test.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm the compile-time sanity assertions (`const _: () = assert!(FLOW_DEPTH_MAX >= 1);` and peers) are present so a nonsensical limit fails the build. Run the `cargo tree` purity check and confirm `tmx-schema` pulls in no async runtime or I/O crate.
  - *Status:* ☐ unverified

- **O4 — Reviewable: read `limits.rs` against the spec limits table and run the matcher round-trip test to confirm the vocabulary is closed and complete.**
  - *Claim:* a reviewer can read `limits.rs` line-by-line against the spec limits table and run the matcher round-trip test to confirm the vocabulary is closed and complete.
  - *Evidence to collect:* open `crates/tmx-schema/src/limits.rs` beside the limits table in [`.specs/04-execution-engine.md#limits`](../../../04-execution-engine.md#limits) and confirm 1:1 correspondence; run `cargo nextest run -p tmx-schema` and observe the matcher round-trip test pass with its closed-vocabulary count assertion.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress.

## Residue

- `CANCEL_GRACE_MS` and `MASK_SCAN_LEN_MIN_BYTES` are named in the task Steps but may sit outside the main [04 limits table](../../../04-execution-engine.md#limits) rows — confirm each has a documented default source (04 §Cancellation / §Masking) before marking O1 SATISFIED.
- The "25" vocabulary size is asserted by the task and spec 05; if the schema's `matcherName` enum count differs on inspection, treat the discrepancy as a finding, not a pass.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
