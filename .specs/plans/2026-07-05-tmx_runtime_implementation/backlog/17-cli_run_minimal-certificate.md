# Done Certificate — Task 17: `tmx run` (the first end-to-end path)

**Task:** [17-cli_run_minimal.md](17-cli_run_minimal.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 17. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 17) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Deliver the `tmx` binary running `tmx run flow.yaml` end to end — load, preflight, execute `exec`/`assert` tasks, print masked final state to stdout, and return the mapped exit code.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-11 runner integration test path and the Task-15 preflight must keep passing; the composition root is the only place concrete adapter types are named and does not change core behaviour.

## Obligations

- **O1 — `tmx run flow.yaml` (any of the four formats) loads, preflights, executes a flow of `exec` and `assert` tasks, and prints the masked final Pipeline state as one JSON object to stdout.**
  - *Claim:* The binary resolves the flow file (any of the four formats), preflights, runs a mixed `exec`/`assert` flow via `RunFlow`, and prints the masked final Pipeline state as a single JSON object to stdout, with human progress on stderr.
  - *Evidence to collect:* Read `crates/tmx-cli/src/{main,args,compose,config}.rs` and `crates/tmx-cli/src/commands/run.rs`. Run `tmx run docs/examples/single-file-flow.yaml` and one other format and observe one JSON object on stdout. Read the composition root in `compose.rs` and confirm the real adapters (loader, resolver, validator, process runner, clock, id generator, scheduler) are wired into `RunFlow`.
  - *Checks:* Resolve the flow-resolution order (`--file/-f` → positional → `$TMX_FLOW` → `./flow.{…}`/`./tmx.{…}` → folder-layout) so the named file is chosen; confirm `compose.rs` is the only site naming concrete adapter types.
  - *Status:* ☐ unverified

- **O2 — The exit code matches the outcome — 0 on success, 1 on a failed `assert`/task, 4 on an unresolved flow — and a requested secret echoed by a task does not appear in stdout.**
  - *Claim:* `exit_code(&RunError)` maps success → 0, a failed `assert`/task → 1, and an unresolved flow → 4; a secret echoed by a task is masked out of stdout.
  - *Evidence to collect:* Read `fn exit_code(&RunError) -> i32` at the `main` seam. Run a passing flow (expect `$?` = 0), a failing-`assert` flow (expect 1), and a missing/unresolved-flow invocation (expect 4). Run a flow whose task echoes a requested secret and grep stdout — expect the secret absent.
  - *Checks:* Resolve the `exit_code` mapping so a failed assert yields 1 and an unresolved flow yields 4; trace that the final-state reporter runs the merged state through the Masker so a requested secret echoed by a task is redacted in stdout.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Run a real `flow.yaml` from the shell, pipe stdout to `jq`, and confirm the printed state, the stderr progress, and the exit code (Reviewable).**
  - *Claim:* A reviewer can run a real flow from the shell, pipe stdout to `jq`, and observe valid JSON final state, progress on stderr (not stdout), and the mapped exit code.
  - *Evidence to collect:* Run `tmx run docs/examples/single-file-flow.yaml | jq .` in the shell; observe `jq` parses stdout as one JSON object, progress text appears on stderr only, and `$?` matches the outcome.
  - *Status:* ☐ unverified

## Regression check

- Task 17 wires 11/15/16 into the `RunFlow` use case: trace that the Task-11 runner integration test path is unchanged — run the Task-11 core integration test (the deterministic multi-task `assert`/`exec` flow over the fakes) and confirm it still passes with the CLI composition added around it.

## Residue

The secret-non-leak check depends on the Masker (Task 09) running over final state — confirm the exercised flow actually requests and echoes a secret so the test is meaningful. Exit codes 3/5/124/130 exist in the full mapping but only 0/1/4 are in this task's DoD; do not fail the task for the others being untested here. stdout must carry only the JSON object — any stray log on stdout breaks the `| jq` contract.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
