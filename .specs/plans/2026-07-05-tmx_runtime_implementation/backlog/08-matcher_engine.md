# Task 08 — MatcherEngine (the shared assert/eval primitive)

**Plan:** [plan.md](../plan.md) · **Certificate:** [08-matcher_engine-certificate.md](08-matcher_engine-certificate.md)

**Implements:** [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §The MatcherEngine, §Scorers (the `matcher` scorer); [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §TaskDispatcher (`assert` is pure)
**Depends on:** 02, 04
**Produces:** the pure, sync, allocation-light implementation of the closed Vitest matcher vocabulary behind both `assert` (gate) and the `matcher` scorer (score)
**Pointers:** `crates/tmx-core/src/matcher.rs` (new), `crates/tmx-schema/src/matcher.rs` (the `MatcherName` enum)

## Steps

- [ ] Implement evaluation of `(actual: &Value, name: MatcherName, expected: Option<&[Value]>, not: bool) -> bool` over the 25 value matchers, handling multi-argument matchers (e.g. `toHaveProperty(path, value)`).
- [ ] Match `MatcherName` exhaustively with no fallthrough, and assert exhaustiveness so the closed enum and the code cannot drift.
- [ ] Expose the boolean result so `assert` aggregates a gate (fail if any assertion does not hold) and the `matcher` scorer maps it to `1.0`/`0.0`, respecting `not`.
- [ ] Add unit tests per matcher (pass and fail, with and without `not`) and property tests over `actual`/`expected` shapes.

## Definition of done

- [ ] Every matcher in the [vocabulary](../../../05-fan-out-and-eval.md#the-matcherengine) returns the correct boolean for representative pass and fail cases, and `not` inverts each.
- [ ] The engine is sync and I/O-free, and a test asserts exhaustiveness over `MatcherName` (negative space: a hypothetical unmatched variant would fail to compile or trip the assertion).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the per-matcher and property tests and confirm `assert` and the `matcher` scorer both consume the same engine.
