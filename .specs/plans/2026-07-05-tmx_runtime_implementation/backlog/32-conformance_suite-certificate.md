# Done Certificate — Task 32: Golden-Flow conformance suite

**Task:** [32-conformance_suite.md](32-conformance_suite.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 32. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 32) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship the workspace-level conformance suite — golden Flows driving `RunFlow` with the
  recorded testkit adapters, asserting the event stream and final state — plus the limit-boundary and
  property-test tier, per [architecture-principles.md](../../../architecture-principles.md) §2.5 and
  [development-guidelines.md](../../../development-guidelines.md) §Testing.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This is a test-only tier over the whole engine — Task 12 (lifecycle hooks),
  Task 15 (preflight), Task 18 (`map`), Task 19 (`eval`), Task 26 (reporters/events), Task 27 (run
  store) and everything they compose. It changes no production code; it asserts existing behaviour.
  Determinism rests on the fixed `Clock`/`IdGenerator`/`SerialScheduler` fakes from `tmx-testkit`.

## Obligations

- **O1 — The golden Flows pass deterministically (byte-identical event stream + final state across two runs) under the fake adapters, covering every task type and lifecycle hook.**
  - *Claim:* each golden Flow drives `RunFlow` with the recorded testkit adapters and asserts the
    recorded event stream plus the final state; two runs produce byte-identical results; coverage
    spans every task type (`exec`, `assert`, `fetch`, `file`, `store`, `chat-completion`, `map`,
    `eval`, `flow` import, `if` skip) and every lifecycle hook (`create`/`change`/`destroy`/`error`).
  - *Evidence to collect:* read the golden-Flow tests under the workspace-level `tests/` and the
    recording adapters in `crates/tmx-testkit/`; confirm the fixtures enumerate each task type and
    each lifecycle hook. Run the conformance tier (`cargo nextest run`) twice and compare the recorded
    event-stream + final-state artifacts byte-for-byte across the two runs — expect identical.
  - *Checks:* trace one golden Flow's recorded event stream against the spec lifecycle
    (create → per-task → destroy) and confirm the ordering matches; confirm determinism is sourced
    only from the fixed `Clock`/`IdGenerator`/`SerialScheduler` fakes (no `SystemTime::now()`, no
    randomness, no `TokioScheduler` in any test body).
  - *Status:* ☐ unverified

- **O2 — Every limit has below/at/above boundary coverage and every required negative-space case (leaked secret, over-cap, too-deep, duplicate name) is present and fails closed (negative space).**
  - *Claim:* `STATE_SIZE_MAX_BYTES`, `FANOUT_WIDTH_MAX`, `FLOW_DEPTH_MAX`, `TASKS_PER_FLOW_MAX`,
    `EXPR_*` (`EXPR_LEN_MAX_BYTES`, `EXPR_DEPTH_MAX`), and `JSON_DEPTH_MAX` each carry a one-below / at
    / one-above test; and the four required negative-space cases — a leaked-secret test, an over-cap
    state test, a too-deep-recursion test, and a duplicate-name test — each exist and fail closed.
  - *Evidence to collect:* read the limit-boundary tests under `tests/` and confirm three cases
    (below, at, above) per named constant; read the four negative-space tests. Run `cargo nextest
    run` and confirm the below/at cases pass and the above/negative cases fail closed with the
    documented typed error (e.g. `state_cap_exceeded`, `fanout_too_wide`) naming the limit.
  - *Checks:* confirm each boundary trio sits exactly one below / at / one above its named constant;
    confirm each negative-space case fails closed — a typed error naming the limit or violation, never
    a panic, a silent truncation, or an unmasked leak.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* the conformance tier (which IS the `cargo nextest run` slow tier) passes, clippy and
    rustfmt are clean, and the golden Flows are byte-identical across two runs — determinism supplied
    by the fixed `Clock`/`IdGenerator`/`SerialScheduler`.
  - *Evidence to collect:* run `cargo nextest run` (the conformance tier), `cargo clippy
    --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run
    `cargo nextest run` a second time and confirm the golden Flows' event stream + final state are
    byte-identical to the first run.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the conformance tier twice and confirm identical results, then inspect one golden Flow's asserted event stream against the spec's lifecycle (Reviewable).**
  - *Claim:* a reviewer can run the conformance tier twice, observe byte-for-byte identical results,
    and read one golden Flow's asserted event stream and see it match the spec lifecycle
    (create → per-task → destroy).
  - *Evidence to collect:* run `cargo nextest run` for the conformance tier twice, capturing the
    recorded event stream + final state each run, and diff the two — expect an empty diff; then open
    one golden Flow's expected event-stream fixture and check it against the
    [04-execution-engine](../../../04-execution-engine.md) lifecycle (create → per-task → destroy).
  - *Status:* ☐ unverified

## Regression check

- No production code changed — the suite asserts existing behaviour; a failing golden Flow indicates a
  regression in the task under test (12/15/18/19/26/27 and everything the engine composes), not in
  Task 32. Golden Flows are marked `#[ignore]` only where a real backend is needed; the CI slow tier
  runs the rest.

## Residue

- The Produces line names a property-test tier alongside the boundary tests — validator should confirm
  the `proptest` cases for the interpolation parser, the matcher engine, and state merge exist and run
  under `nextest`, per development-guidelines §Testing.
- CI wiring as the slow tier is part of the task — confirm the conformance tier is invoked in CI (not
  only locally) and that the `#[ignore]` gating is limited to real-backend cases.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
