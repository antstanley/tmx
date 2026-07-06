# Done Certificate — Task 32: Golden-Flow conformance suite

**Task:** [32-conformance_suite.md](32-conformance_suite.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — discharged by the verifier (all four obligations SATISFIED)

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
  - *Status:* ☑ SATISFIED — 11 golden Flows in `crates/tmx-conformance/tests/golden_flows.rs`; the
    `golden` helper runs every Flow twice over two fresh bundles and asserts byte-identical NDJSON
    event stream + final state + run id. Coverage verified by reading: exec, assert, fetch, file,
    store, chat-completion, flow import, if-skip through `EngineRunFlow`; map/eval through
    `run_map`/`run_eval` over `SerialScheduler` (the sequential runner rejects map/eval with
    `task_type_unsupported` by design — dispatch.rs:117 — so the fan-out functions ARE the real
    path). Hooks create/change/destroy/error all asserted; flagship test pins the full lifecycle
    stream. Grep confirms no SystemTime/rand/TokioScheduler in any test. Note: the `run` task type
    (exec's interpreter twin, same ProcessRunner path) is not in this enumeration and has no golden
    Flow — an advisory gap, not an obligation breach.

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
  - *Status:* ☑ SATISFIED — `tests/limit_boundaries.rs` carries a below/at/above trio for all seven
    constants, each computed FROM the `tmx-schema::limits` constant (verified by reading; no
    hard-coded limit literals). Above cases assert the documented typed errors:
    `state_cap_exceeded`, `json_too_deep`, `flow_depth_exceeded`, `too_many_tasks`,
    `fanout_too_wide`, `expr_too_long`, `expr_too_deep`. STATE_SIZE (512 MiB, impractical to
    materialise) is probed byte-exactly against a narrowed configured cap through the same
    `StateBuilder::with_cap` guard and separately pinned to the named constant (default = ceiling,
    over-ceiling clamped) — the merge.rs pattern. All four negative-space cases present in
    `tests/negative_space.rs` and pass; the over-cap guard was verified live by widening the cap
    (test failed as it must) and reverting (test passed). Leaked-secret case asserts both absence
    of the raw value AND presence of `[REDACTED]` in state and NDJSON stream.

- **O3 — Meets the repo definition of done.**
  - *Claim:* the conformance tier (which IS the `cargo nextest run` slow tier) passes, clippy and
    rustfmt are clean, and the golden Flows are byte-identical across two runs — determinism supplied
    by the fixed `Clock`/`IdGenerator`/`SerialScheduler`.
  - *Evidence to collect:* run `cargo nextest run` (the conformance tier), `cargo clippy
    --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run
    `cargo nextest run` a second time and confirm the golden Flows' event stream + final state are
    byte-identical to the first run.
  - *Status:* ☑ SATISFIED — independently run by the verifier from the repo root:
    `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings`
    clean; `cargo nextest run` 447/447 passed (26 new conformance tests); `cargo build` clean;
    `scripts/purity.sh` green (proptest + transitive deps confined to tmx-conformance;
    tmx-schema/tmx-core/tmx-testkit unchanged). Determinism is asserted inside every golden test
    (two fresh bundles per test, byte-identical streams), and the tier as a whole passed twice.

- **O4 — Reviewable: run the conformance tier twice and confirm identical results, then inspect one golden Flow's asserted event stream against the spec's lifecycle (Reviewable).**
  - *Claim:* a reviewer can run the conformance tier twice, observe byte-for-byte identical results,
    and read one golden Flow's asserted event stream and see it match the spec lifecycle
    (create → per-task → destroy).
  - *Evidence to collect:* run `cargo nextest run` for the conformance tier twice, capturing the
    recorded event stream + final state each run, and diff the two — expect an empty diff; then open
    one golden Flow's expected event-stream fixture and check it against the
    [04-execution-engine](../../../04-execution-engine.md) lifecycle (create → per-task → destroy).
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-conformance` run twice by the verifier:
    26/26 pass both runs, identically. The flagship
    `golden_exec_and_assert_with_create_and_destroy_hooks` asserts the exact stream
    run.start → hook(create, bracketing its own exec) → per-task → hook(destroy) → run.finish,
    matching 04-execution-engine's create → per-task → destroy order (destroy before run.finish,
    per spec step 3; create fires immediately after run.start per the runner's documented design —
    runner.rs:255 — a pre-existing task-12 ordering, faithfully pinned).

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
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED on independently-run evidence: 26 deterministic conformance
tests (11 golden Flows asserting byte-identical two-run streams/state/ids over the testkit fakes,
7 below/at/above limit trios computed from the named tmx-schema constants, 4 fail-closed
negative-space cases, 3 proptest properties with persistence disabled), all green under
fmt/clippy/nextest (447/447) and the purity gate; the residue items check out — proptest covers the
three named services and the tier runs in gate.sh's workspace `cargo nextest run` (the repo's CI
surface; no `#[ignore]` anywhere since no case needs a real backend). No production code changed.
Advisory (non-blocking): the `run` task type (exec's interpreter twin over the same ProcessRunner
path) has no golden Flow of its own; the spec prose of 04-execution-engine step 1 lists the create
hook before `run.start` while the engine (and the golden) order run.start first — a pre-existing
task-12/spec wording nuance, not a task-32 defect.
