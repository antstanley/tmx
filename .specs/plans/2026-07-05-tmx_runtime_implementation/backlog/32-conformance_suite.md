# Task 32 — Golden-Flow conformance suite

**Plan:** [plan.md](../plan.md) · **Certificate:** [32-conformance_suite-certificate.md](32-conformance_suite-certificate.md)

**Implements:** [architecture-principles.md](../../../architecture-principles.md) §2.5 Determinism and testability (Golden Flows); [development-guidelines.md](../../../development-guidelines.md) §Testing (Test pyramid), §Definition of done; [00-overview.md](../../../00-overview.md) §Goals (the conformance basis)
**Depends on:** 12, 15, 18, 19, 26, 27
**Produces:** the workspace-level conformance suite — golden Flows driving `RunFlow` with recorded adapters, asserting the event stream and final state — plus the limit-boundary and property-test tier
**Pointers:** `tests/` (workspace-level, new), `crates/tmx-testkit/` (recording adapters), CI config (the conformance tier)

## Steps

- [ ] Author golden Flows covering each task type and the lifecycle (create/change/destroy/error, `if` skip, `map`, `eval`, `flow` import), each driving `RunFlow` with the recorded testkit adapters and asserting the recorded event stream plus the final state.
- [ ] Add limit-boundary tests for each tunable and structural limit — one below, at, and one above (`STATE_SIZE_MAX_BYTES`, `FANOUT_WIDTH_MAX`, `FLOW_DEPTH_MAX`, `TASKS_PER_FLOW_MAX`, `EXPR_*`, `JSON_DEPTH_MAX`).
- [ ] Add the negative-space conformance cases the guidelines require: a leaked-secret test, an over-cap-state test, a too-deep-recursion test, and a duplicate-name test.
- [ ] Wire the suite into CI as the slow tier (golden Flows marked `#[ignore]` only where a real backend is needed), keeping every case deterministic via the fixed `Clock`/`IdGenerator`/`SerialScheduler`.

## Definition of done

- [ ] The golden Flows pass deterministically (byte-identical event stream + final state across two runs) under the fake adapters, covering every task type and lifecycle hook.
- [ ] Every limit has below/at/above boundary coverage and every required negative-space case (leaked secret, over-cap, too-deep, duplicate name) is present and fails closed.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the conformance tier twice and confirm identical results, then inspect one golden Flow's asserted event stream against the spec's lifecycle.
