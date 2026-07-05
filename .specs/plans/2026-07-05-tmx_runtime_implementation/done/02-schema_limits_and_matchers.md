# Task 02 — Schema limits and the matcher vocabulary

**Plan:** [plan.md](../plan.md) · **Certificate:** [02-schema_limits_and_matchers-certificate.md](02-schema_limits_and_matchers-certificate.md)

**Implements:** [04-execution-engine.md](../../../04-execution-engine.md) §Limits; [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §The MatcherEngine (the `MatcherName` enum); [02-crate-architecture.md](../../../02-crate-architecture.md) §tmx-schema (`limits.rs`, `matcher.rs`); [development-guidelines.md](../../../development-guidelines.md) §Limits and bounds
**Depends on:** 01
**Produces:** the single source-of-truth `tmx-schema::limits` module holding every named units-last constant, plus the closed `MatcherName` enum, each backed by compile-time sanity assertions
**Pointers:** `crates/tmx-schema/src/limits.rs` (new), `crates/tmx-schema/src/matcher.rs` (new), `crates/tmx-schema/src/lib.rs`

## Steps

- [x] Define every limit from the [limits table](../../../04-execution-engine.md#limits) as a named units-last constant with its unit in the identifier: `STATE_SIZE_MAX_BYTES`, `FLOW_DEPTH_MAX`, `TASKS_PER_FLOW_MAX`, `FANOUT_WIDTH_MAX`, `CONCURRENCY_MAX`, `EXPR_LEN_MAX_BYTES`, `EXPR_DEPTH_MAX`, `JSON_DEPTH_MAX`, `CAPTURED_OUTPUT_MAX_BYTES`, `HOOK_TASKS_MAX`, `EVENT_LOG_MAX_BYTES`, `CANCEL_GRACE_MS`, `MASK_SCAN_LEN_MIN_BYTES`, each with a doc comment stating what it bounds and where it is enforced.
- [x] Add compile-time sanity assertions (`const _: () = assert!(FLOW_DEPTH_MAX >= 1);` and peers) so a nonsensical limit fails the build, and mark which constants are config-tunable vs structurally fixed.
- [x] Define `MatcherName` as a closed enum over the 25 value matchers of the schema `matcherName` vocabulary (mock/promise matchers excluded), deriving serde with the schema's string spellings.
- [x] Re-export `limits` and `MatcherName` from `lib.rs`; use explicit fixed-width integer types for the constants and avoid `usize` for any value that crosses a serialization boundary.

## Definition of done

- [x] Every constant in the [limits table](../../../04-execution-engine.md#limits) exists with the exact units-last name and its documented default value, referenced nowhere as a magic number.
- [x] `MatcherName` round-trips every schema spelling, and a unit test asserts the variant count equals the closed vocabulary size (negative space: an added or dropped matcher fails the test).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: read `limits.rs` against the spec limits table and run the matcher round-trip test to confirm the vocabulary is closed and complete.
