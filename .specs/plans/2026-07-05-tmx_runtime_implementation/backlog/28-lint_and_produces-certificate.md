# Done Certificate — Task 28: `lint` and `produces` conformance

**Task:** [28-lint_and_produces.md](28-lint_and_produces.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 28. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 28) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship `LintFlow` — the deeper static pass (resolution + dataflow) — plus the opt-in runtime `produces` conformance check, behind `tmx lint` and `--check-produces`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Task-15 preflight and Task-14 schema validation still pass a valid Flow at their shallower depth — `validate` (pure schema) is unchanged; both `validate` and `lint` exit 3 but at different depths; the Task-11 runner's `produces` check hook is the seam the runtime flag wires into, not a new dispatch path.

## Obligations

- **O1 — `tmx lint` catches a typo'd `produces` read, an undeclared input, an unlisted secret, and a cyclic import, and `--strict` turns each warning into an exit-3 error.**
  - *Claim:* `lint` resolves `environment`/`context`/`flow` references and confirms they load; walks every `${{ tasks.NAME.field }}` against the referenced task's `produces` schema (catching `tasks.build.artifcat`); flags inputs used-but-undeclared and secrets used-but-unlisted; flags duplicate/missing array-form names; detects cyclic `flow` imports; `--strict` promotes each warning to an exit-3 error.
  - *Evidence to collect:* read `crates/tmx-core/src/usecases.rs` (`LintFlow`) for the resolution + dataflow passes and `crates/tmx-cli/src/commands/lint.rs` for `--strict`; run the named lint tests seeding, respectively, a typo'd `produces` read (`tasks.build.artifcat`), an undeclared input, an unlisted secret, and a cyclic import — each expected to emit a `Diagnostic`; rerun under `--strict` and expect exit 3.
  - *Checks:* resolve a `${{ tasks.NAME.field }}` read to the referenced task's `produces` schema and confirm the typo'd field surfaces as a `Diagnostic` rather than silently resolving; confirm `lint` reaches this depth while `validate` (pure schema) does not.
  - *Status:* ☐ unverified

- **O2 — `--check-produces=strict` fails a violating task, a bare `--check-produces` only warns, and an absent flag checks nothing.**
  - *Claim:* the runtime `produces` check has three states — `--check-produces=strict` fails a task whose output violates its `produces` schema, a bare `--check-produces` (`=warn`) emits a warning `Diagnostic` and continues, and an absent flag skips the check entirely.
  - *Evidence to collect:* read `crates/tmx-core/src/runner.rs` for the `produces` check hook wired to `--check-produces[=warn|strict]`; run three tests over a task whose output violates its `produces` schema — one with `=strict` (task fails), one with a bare `--check-produces`/`=warn` (warning, run continues), one with no flag (task runs, no check) — and confirm the three outcomes differ.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Reviewable: `tmx lint` on a seeded dataflow defect, then `--check-produces=warn` and `=strict` with differing outcomes.**
  - *Claim:* a reviewer can run `tmx lint` on a Flow carrying a seeded dataflow defect and observe the `Diagnostic` (and exit 3 under `--strict`), then run the same Flow with `--check-produces=warn` (a warning, run proceeds) and `--check-produces=strict` (the task fails) and observe the differing outcomes.
  - *Evidence to collect:* run `tmx lint flow.yaml` on a fixture with a seeded dataflow defect (e.g. the typo'd `produces` read) and observe the `Diagnostic` and exit code; run the same flow under `--check-produces=warn` and `--check-produces=strict` and confirm the warn-vs-fail difference.
  - *Status:* ☐ unverified

## Regression check

- Task-15 preflight / Task-14 validation: trace that `tmx validate flow.yaml` (pure schema) still passes a valid Flow at exit 0 — the added `LintFlow` resolution+dataflow depth does not change `validate`'s shallower schema-only pass.

## Residue

- The static `produces` walk (lint) and the runtime `produces` check (runner hook) share the `produces` schema but at different times; confirm a Flow that lints clean can still trip `--check-produces=strict` at runtime and vice versa.
- `environment.options` vs a provider `optionsSchema` is validated in both Task 15 (preflight) and here (lint); confirm the lint diagnostic and the preflight rejection agree rather than diverge.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
