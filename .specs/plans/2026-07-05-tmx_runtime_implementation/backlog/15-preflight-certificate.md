# Done Certificate — Task 15: Preflight (load → resolve → validate → capability)

**Task:** [15-preflight.md](15-preflight.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 15. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 15) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Deliver the `tmx-core` preflight orchestration that either passes wholesale to a `ResolvedFlow` + `CapabilitySet` or fails fast with nothing executed.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-13 loader/resolver and Task-14 validator this composes must keep passing their tests; a valid corpus Flow that preflighted before still preflights; `tmx-core` stays pure (Task-01 purity gate).

## Obligations

- **O1 — A passing preflight yields a `ResolvedFlow` (ordered `Vec<Task>`, resolved env/context) and a `CapabilitySet`; a directory with one malformed task aborts before any task executes.**
  - *Claim:* A valid Flow (single-file or directory) preflights to a `ResolvedFlow` with an ordered `Vec<Task>` and resolved env/context plus a `CapabilitySet`; a directory containing one malformed task aborts in preflight with nothing executed.
  - *Evidence to collect:* Read `crates/tmx-core/src/preflight.rs`, the desugar helpers in `crates/tmx-schema/src/`, and `ValidateArtifacts`/`InspectFlow` in `crates/tmx-core/src/usecases.rs`. Preflight `docs/examples/folder-layout/` (`context.yaml`, `environment.toml`, `task-1.jsonc`, `task-2.yaml`) and expect a `ResolvedFlow` + `CapabilitySet`. Run the malformed-task-in-a-directory test and expect an abort before any task runs.
  - *Checks:* Trace a directory of task files through natural-filename ordering and confirm `task-2` precedes `task-10` (maximal digit runs compared as unsigned integers, byte-wise ASCII, case-sensitive); confirm the malformed-task abort happens in preflight, before the runner is invoked (nothing executed).
  - *Status:* ☐ unverified

- **O2 — Over-limit counts/widths/depths and a nameless or duplicate-named task are rejected at preflight, and a Flow needing an unwired capability returns `missing_capability` naming the port and task type.**
  - *Claim:* The preflightable limits (`too_many_tasks`, literal `fanout_too_wide`, `json_too_deep`, `concurrency_too_high`, `too_many_hook_tasks`) and the structural checks (`missing_task_name`, duplicate-name `ResolutionError`) reject at preflight; a Flow requiring an absent or denying-stub adapter returns `EnvironmentError`/`missing_capability` naming the port and task type.
  - *Evidence to collect:* Read the limit and capability computation in `crates/tmx-core/src/preflight.rs`. Run the over-limit tests (counts / widths / depths) and the nameless and duplicate-name tests and expect `Validation` / `ResolutionError`. Run the missing-capability test and expect `missing_capability` naming the port and task type.
  - *Checks:* Trace the capability computation recursing into `map`/`eval` inner tasks, `eval` scorer kinds, hook bodies, and provider methods, so a required-but-unwired adapter surfaces as `missing_capability` naming the port and task type.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant, and `tmx-core` stays pure.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm any new limit is a named units-last constant in `tmx-schema::limits`. Run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio` finds nothing) confirming preflight adds no impure edge into `tmx-core`.
  - *Status:* ☐ unverified

- **O4 — Preflight a directory of task files and confirm natural-filename ordering, and run the fail-fast tests for a malformed task and a missing capability (Reviewable).**
  - *Claim:* A reviewer can preflight a directory and observe tasks in natural-filename order with `environment.*`/`context.*` folded into shared context, and run the two fail-fast tests observing abort / typed errors.
  - *Evidence to collect:* Preflight `docs/examples/folder-layout/` and observe `task-1` before `task-2` with the sibling `environment.*`/`context.*` folded into shared context; run the malformed-task and missing-capability tests via `cargo nextest run` and observe fail-fast.
  - *Status:* ☐ unverified

## Regression check

- Preflight composes the Task-13 loader/resolver and Task-14 validator: trace that a valid corpus Flow still preflights — run the preflight over `docs/examples/single-file-flow.yaml` and the `docs/examples/folder-layout/` directory and confirm each still yields a `ResolvedFlow` after the loader/validator are composed behind it.

## Residue

Natural ordering is byte-wise ASCII, case-sensitive, with maximal digit runs compared as unsigned integers — spot-check a mixed-case and a leading-zero case; the corpus `folder-layout/` has only `task-1`/`task-2`, so the `task-2` < `task-10` semantic may need a constructed fixture. The newer-spec compatibility warning is emitted (non-fatal); an unknown construct is fatal `Validation`. Directory context assembly is same-folder only.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
