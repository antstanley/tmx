# Done Certificate — Task 08: MatcherEngine (the shared assert/eval primitive)

**Task:** [08-matcher_engine.md](08-matcher_engine.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

> This certificate is a verification protocol for Task 08. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 08) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion. O4 is the
Reviewable item; record DONE only when O1…O4 are all SATISFIED.

## Premises

- **P1 — Goal.** Produce the pure, sync, allocation-light implementation of the closed Vitest matcher
  vocabulary behind both `assert` (gate) and the `matcher` scorer (score).
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This is a new pure-core unit. It builds on the closed `MatcherName` enum from
  Task 02 in `crates/tmx-schema/src/matcher.rs` and the Task 04 `Value` type; those must keep
  compiling and passing. No task consumes the engine yet — Task 11 (`assert`) and Task 19 (the
  `matcher` scorer) will — so there is no in-scope caller to regress.

## Obligations

- **O1 — Every matcher in the vocabulary returns the correct boolean for representative pass and fail cases, and `not` inverts each.**
  - *Claim:* evaluating `(actual, name, expected, not)` over the 25 value matchers
    ([05 §The MatcherEngine](../../../05-fan-out-and-eval.md#the-matcherengine)) yields the correct
    boolean for a representative passing and failing input each, including multi-argument matchers
    (e.g. `toHaveProperty(path, value)`), and setting `not: true` inverts every result.
  - *Evidence to collect:* read the engine in `crates/tmx-core/src/matcher.rs`; run
    `cargo nextest run -p tmx-core matcher` and confirm the per-matcher unit tests (pass and fail,
    with and without `not`) pass for all 25 matchers, and the property tests over `actual`/`expected`
    shapes pass.
  - *Checks:* trace a multi-argument matcher (`toHaveProperty(path, value)`) and confirm both the
    path lookup and the value comparison are consumed from `expected: &[Value]`; trace one matcher
    with `not: true` and confirm the inversion wraps the base result rather than being a separate
    arm.
  - *Status:* ☑ SATISFIED — All 25 arms read once (matcher.rs:87-177); each has a per-matcher
    pass+fail unit test (`to_be_is_shallow_object_is` … `throw_and_satisfy_read_a_pre_resolved_result`).
    `toHaveProperty(path, value)` traced: `has_property` reads `arg(expected,0)` for the path lookup
    and `arg(expected,1)` for the deep-equal value check (matcher.rs:335-346), exercised by
    `have_property_paths_and_values` (dotted, bracket-indexed, array-of-segments, mismatched-value,
    absent-path). `not` is a single XOR over the base result (`evaluate_base(...) ^ not`,
    matcher.rs:80), NOT a per-matcher arm; `property_not_always_inverts_over_random_inputs`
    (3000 inputs × 25) confirms `base == !negated` for every variant. `cargo nextest run -p tmx-core
    matcher` → 18 passed, 0 failed.

- **O2 — The engine is sync and I/O-free, and a test asserts exhaustiveness over `MatcherName` so an unmatched variant would fail to compile or trip the assertion.**
  - *Claim:* the engine takes no port, awaits nothing, and does no I/O; the `match` over
    `MatcherName` has no `_ =>` fallthrough, and a hypothetical unmatched variant fails to compile or
    trips an exhaustiveness assertion so the closed enum and the code cannot drift.
  - *Evidence to collect:* read `crates/tmx-core/src/matcher.rs` and confirm the evaluation function
    is a plain sync `fn` (no `async`, no port parameter); read the `match name { … }` and confirm no
    wildcard arm; run `cargo nextest run -p tmx-core matcher` and confirm the exhaustiveness test
    (iterating every `MatcherName` variant, or a compile-time assertion) passes.
  - *Checks:* resolve the dispatch site and confirm every `MatcherName` variant has an explicit arm
    (no `_`), so adding a variant to the Task 02 enum forces a non-exhaustive-match compile error
    here.
  - *Status:* ☑ SATISFIED — `evaluate` is a plain sync `fn`, no `async`, no port parameter, no I/O
    (matcher.rs:73-81). The `match matcher` at matcher.rs:87 has NO `_` wildcard; all 25 variants are
    explicit (inner `_` arms are on `Value`/constructor-name, not on `MatcherName`). Verified the
    guard is real: injected a probe variant into the Task 02 enum → `cargo build -p tmx-core` failed
    with `error[E0004]: non-exhaustive patterns: MatcherName::ToProbeExhaustiveness not covered` at
    matcher.rs:87:15; probe reverted, tree rebuilds green. Runtime companion
    `every_matcher_variant_is_dispatched_without_panic` iterates `MatcherName::ALL` and asserts
    dispatched count == `MatcherName::COUNT` (25). Purity gate green; `cargo tree -p tmx-core -i
    tokio` → no such package (no async edge).

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm any bound introduced is a
    named `tmx-schema::limits` constant (this task adds no new numeric limit — confirm none was
    magic-numbered); run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio`
    expecting no match) confirming `tmx-core` takes on no async-runtime/I/O edge.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` clean; `cargo clippy --all-targets
    --all-features -- -D warnings` → exit 0, no warnings; `cargo nextest run` → 76 passed, 0 failed.
    No new `tmx-schema::limits` entry (limits.rs untouched; the code diff is only lib.rs + matcher.rs).
    The only numeric bounds introduced are `toBeCloseTo`'s Vitest algorithm constants
    (`CLOSE_TO_DEFAULT_PRECISION_DIGITS`, `CLOSE_TO_PRECISION_BASE`, `CLOSE_TO_TOLERANCE_HALVING`,
    matcher.rs:52-56) — named locals, correctly NOT engine dimensions, so the tolerance formula reads
    with no bare magic literal. `scripts/purity.sh` green; `cargo tree -p tmx-core -i tokio` → no such
    package (no async/I/O edge).

- **O4 — Reviewable: run the per-matcher and property tests and confirm `assert` and the `matcher` scorer both consume the same engine (Reviewable).**
  - *Claim:* a reviewer can run the matcher test suite and observe the per-matcher and property tests
    pass, and can read the code to confirm a single `MatcherEngine` is the primitive both `assert`
    and the `matcher` scorer are wired to.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core matcher` and read the summary for zero
    failures; read the engine's public surface and confirm it exposes one boolean-returning entry
    point (no `assert`-specific and separate scorer-specific copy), matching the "shared primitive"
    contract in [05 §Scorers](../../../05-fan-out-and-eval.md#scorers).
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core matcher` → 18 passed, 0 failed (per-matcher
    unit tests + `property_not_always_inverts_over_random_inputs` + `property_deep_equality_is_reflexive`
    + `every_matcher_variant_is_dispatched_without_panic`). Public surface is a single ZST
    `MatcherEngine` with one boolean-returning associated fn `evaluate(actual, matcher, expected, not)
    -> bool` (matcher.rs:64-81) — no assert-specific vs scorer-specific copy. Greenfield: the only
    reference to `MatcherEngine` outside its module is `pub use matcher::MatcherEngine` in lib.rs:32;
    Task 11 (`assert` gate) and Task 19 (`matcher` scorer) will both call this one primitive, matching
    the [05 §Scorers] shared-primitive contract.

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The Task 02 `MatcherName` enum and
  the Task 04 `Value` type this unit imports must still compile and pass.

## Residue

- Confirm the matcher count is exactly 25 and matches the schema's `matcherName` enum (mock/promise
  matchers excluded per the schema) — a drift here would not fail O1 if a test omits the extra
  variant, so cross-check the enum, not only the tests.
- Confirm equality matchers treat `NaN`, deep-vs-shallow equality (`toBe` vs `toEqual`), and numeric
  tolerance (`toBeCloseTo`) per Vitest semantics; these are inside item 1's "correct boolean" but are
  the likeliest subtle miss.
- The allocation-light property is a Step, not a DoD item; note it for review but it does not gate.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED. All 25 closed matchers implemented over the JSON value model
with a single sync, I/O-free, panic-free `MatcherEngine::evaluate` primitive; `not` is one XOR over
the base result. Exhaustiveness guard empirically verified (probe variant → E0004 at matcher.rs:87,
then reverted). fmt/clippy/nextest all clean (76 passed), purity gate green, no tokio edge, no new
limit. `toBeCloseTo` matches Vitest's `10^-precision/2` tolerance with named constants. Regression
check: greenfield — the only external reference is `pub use` in lib.rs; Task 02 `MatcherName` and
Task 04 `Value` untouched and still pass. Residual live-JS divergences (`instanceof`/`toHaveLength`
on objects, `toStrictEqual(-0,0)`) are documented, defensible JSON-model choices unpinned by the spec
and non-gating.
