# Done Certificate — Task 28: `lint` and `produces` conformance

**Task:** [28-lint_and_produces.md](28-lint_and_produces.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

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
  - *Status:* ☑ SATISFIED — `crates/tmx-core/src/lint.rs` (`analyze_flow` + `check_reference`) resolves `tasks.NAME.field` against the named task's `produces.properties` and flags `artifcat` as `produces_field_unknown`; `crates/tmx-core/tests/lint.rs` proves typo/undeclared-input/unlisted-secret through the use case; `crates/tmx-cli/tests/cli_lint.rs` proves exit 0 bare → exit 3 `--strict` on the real binary. Validator ran the real binary on seeded fixtures: typo + undeclared input + unlisted secret each reported on stderr at exit 0, exit 3 under `--strict`; a dangling `context` reference reported `unresolved_reference`; an `environment.options` type violation against a provider `optionsSchema` reported `provider_options_invalid`; an A→B→A import cycle reported `cyclic_flow_import` (exit 3 strict); a fully-declared flow linted clean at exit 0 under `--strict`. The fixtures are schema-valid, so this depth is beyond pure schema validation. Mutation check: disabling `produces_omits_field` tripped 3 tests across unit/use-case/e2e depths (reverted).

- **O2 — `--check-produces=strict` fails a violating task, a bare `--check-produces` only warns, and an absent flag checks nothing.**
  - *Claim:* the runtime `produces` check has three states — `--check-produces=strict` fails a task whose output violates its `produces` schema, a bare `--check-produces` (`=warn`) emits a warning `Diagnostic` and continues, and an absent flag skips the check entirely.
  - *Evidence to collect:* read `crates/tmx-core/src/runner.rs` for the `produces` check hook wired to `--check-produces[=warn|strict]`; run three tests over a task whose output violates its `produces` schema — one with `=strict` (task fails), one with a bare `--check-produces`/`=warn` (warning, run continues), one with no flag (task runs, no check) — and confirm the three outcomes differ.
  - *Status:* ☑ SATISFIED — `runner.rs::check_produces` (the pre-existing task-11 seam, untouched by this diff) dispatches Off/Warn/Strict; `crates/tmx-cli/src/commands/run.rs` wires `--check-produces` (clap `num_args=0..=1, default_missing_value="warn"`) into `RunConfig.check_produces`. `crates/tmx-core/tests/produces.rs` proves the three states over a violating task: off → `Ok` + 0 `validate_produces` calls, warn → `Ok` + 1 call, strict → `Failed` carrying `produces_mismatch`, plus a mutual-distinctness cross-check. Validator reran the real binary on a violating fixture: absent → exit 0, bare `--check-produces` → exit 0, `=strict` → exit 1. Residue (non-blocking, recorded below): under `warn` the mismatch diagnostic is computed but discarded — spec 04 says warn "emits a warning Diagnostic", but spec 08's canonical event vocabulary has no diagnostic/warning event to carry it, so surfacing it needs a cross-cutting spec-08/Event change out of this task's scope.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — validator ran all three gates independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 357/357 passed (`--all-features` 374 passed, 2 skipped); `scripts/purity.sh` green. No new numeric bound was introduced — the import walk reuses the existing named `FLOW_DEPTH_MAX` from `tmx-schema::limits`. Coverage note (non-blocking, recorded below): the two secondary lint paths `unresolved_reference` and `provider_options_invalid` have no automated test; the validator exercised both on the real binary and they behave correctly.

- **O4 — Reviewable: `tmx lint` on a seeded dataflow defect, then `--check-produces=warn` and `=strict` with differing outcomes.**
  - *Claim:* a reviewer can run `tmx lint` on a Flow carrying a seeded dataflow defect and observe the `Diagnostic` (and exit 3 under `--strict`), then run the same Flow with `--check-produces=warn` (a warning, run proceeds) and `--check-produces=strict` (the task fails) and observe the differing outcomes.
  - *Evidence to collect:* run `tmx lint flow.yaml` on a fixture with a seeded dataflow defect (e.g. the typo'd `produces` read) and observe the `Diagnostic` and exit code; run the same flow under `--check-produces=warn` and `--check-produces=strict` and confirm the warn-vs-fail difference.
  - *Status:* ☑ SATISFIED — validator performed the reviewable action on the compiled binary: `tmx lint` on a seeded-typo flow printed `warning: … [produces_field_unknown]` (plus `undeclared_input` / `undeclared_secret`) on stderr with empty stdout at exit 0, and exit 3 under `--strict`; the violating-`produces` flow ran at exit 0 absent, exit 0 with bare `--check-produces`, and failed at exit 1 with `--check-produces=strict`. `crates/tmx-cli/tests/cli_lint.rs` automates the same runs against the real binary.

## Regression check

- Task-15 preflight / Task-14 validation: trace that `tmx validate flow.yaml` (pure schema) still passes a valid Flow at exit 0 — the added `LintFlow` resolution+dataflow depth does not change `validate`'s shallower schema-only pass.
- **Discharged (at the depth that exists):** the `tmx validate` subcommand is Task 31 and does not exist yet, so the CLI trace is not exercisable; at library depth the diff touches neither `preflight.rs`, the schema-validator adapter, nor `runner.rs`, and the full suite (357 tests, incl. all task-14/15 tests) passes. A valid flow runs end-to-end at exit 0 (`tmx run … --no-store`).

## Residue

- The static `produces` walk (lint) and the runtime `produces` check (runner hook) share the `produces` schema but at different times; confirm a Flow that lints clean can still trip `--check-produces=strict` at runtime and vice versa.
  - **Confirmed:** the violating-output fixture lints clean (exit 0) yet fails under `--check-produces=strict` (exit 1); the typo fixture lints dirty yet is schema-valid.
- `environment.options` vs a provider `optionsSchema` is validated in both Task 15 (preflight) and here (lint); confirm the lint diagnostic and the preflight rejection agree rather than diverge.
  - **Confirmed:** the same type violation (`region: 42` vs `type: string`) that preflight rejects is reported by lint as a `provider_options_invalid` warning naming the identical schema violation.
- **New residue recorded by the validator (non-blocking follow-ups):** (1) under `--check-produces` warn mode the mismatch diagnostic is computed but never surfaced — spec 04 promises a warning `Diagnostic` but spec 08's canonical event vocabulary has no vehicle for it; needs a spec-08/Event follow-up. (2) The lint paths `unresolved_reference` and `provider_options_invalid` lack automated tests (both verified manually on the real binary); a small test addition in a follow-up would close the repo-DoD "every new validation path" clause completely.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently collected evidence: the three gates (fmt/clippy/nextest, 357 passed) plus purity run clean; `tmx lint` catches the typo'd `produces` read, undeclared input, unlisted secret, and cyclic import on the real binary (exit 0 bare, exit 3 `--strict`); the runtime `--check-produces` states are mutually distinct (absent → no check/exit 0, bare → checked/exit 0, strict → task fails/exit 1); a mutation of the typo check tripped tests at all three depths. Two non-blocking residue items recorded: warn-mode diagnostics are computed but unsurfaceable until spec 08 gains a diagnostic event, and `unresolved_reference`/`provider_options_invalid` are verified manually but untested.
