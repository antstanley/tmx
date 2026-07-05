# Task 28 — `lint` and `produces` conformance

**Plan:** [plan.md](../plan.md) · **Certificate:** [28-lint_and_produces-certificate.md](28-lint_and_produces-certificate.md)

**Implements:** [03-loading-and-preflight.md](../../../03-loading-and-preflight.md) §`lint` (static analysis beyond schema); [04-execution-engine.md](../../../04-execution-engine.md) §`produces` conformance; [07-cli.md](../../../07-cli.md) §Command → use case mapping (`lint`)
**Depends on:** 14, 15
**Produces:** `LintFlow` — the deeper static pass (resolution + dataflow) — plus the opt-in runtime `produces` conformance check, behind `tmx lint` and `--check-produces`
**Pointers:** `crates/tmx-core/src/usecases.rs` (`LintFlow`), `crates/tmx-core/src/runner.rs` (the `produces` check hook), `crates/tmx-cli/src/commands/lint.rs` (new)

## Steps

- [ ] Implement `lint`: resolve `environment`/`context`/`flow` references and confirm they load; walk every `${{ tasks.NAME.field }}` against the referenced task's `produces` schema (catching `tasks.build.artifcat`); flag inputs used-but-undeclared and secrets used-but-unlisted; flag duplicate/missing array-form names; detect cyclic `flow` imports; validate `environment.options` against a provider `optionsSchema`.
- [ ] Emit `Diagnostic`s and let `--strict` promote warnings to errors; both `validate` and `lint` exit 3 but at different depths.
- [ ] Implement the runtime `produces` check: with `--check-produces[=warn|strict]`, validate each task output against its `produces` schema — `warn` emits a warning `Diagnostic`, `strict` fails the task, absent skips the check; `lint` uses `produces` statically regardless.
- [ ] Wire `tmx lint` to `LintFlow` and the run-time flag into the runner's conformance step.

## Definition of done

- [ ] `tmx lint` catches a typo'd `produces` read, an undeclared input, an unlisted secret, and a cyclic import, and `--strict` turns each warning into an exit-3 error.
- [ ] `--check-produces=strict` fails a task whose output violates its `produces` schema while a bare `--check-produces` only warns and an absent flag checks nothing (negative space across the three states).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `tmx lint` on a Flow with a seeded dataflow defect, then run it with `--check-produces=warn` and `=strict` and confirm the differing outcomes.
