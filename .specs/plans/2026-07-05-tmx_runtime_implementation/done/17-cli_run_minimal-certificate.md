# Done Certificate — Task 17: `tmx run` (the first end-to-end path)

**Task:** [17-cli_run_minimal.md](17-cli_run_minimal.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — discharged by the verifier (independent of the implementer)

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
  - *Status:* ☑ SATISFIED — Read all five CLI modules. Ran the real binary over fresh fixtures in **all four formats** (yaml, json via `cli_run.rs` + jsonc + toml live): each printed one JSON object to stdout (`jq` parsed it; `.build.message == "built-ok"`, `.check.passed == true`) with per-task progress on stderr only. Resolution order exercised live: explicit positional, `--file` precedence (unit test), `$TMX_FLOW` from an unrelated cwd, no-arg `./flow.yaml` cwd search, explicit directory, and the folder-layout fallback (`environment.yaml` + task files, no arg) — all chose the right target; an exec→assert flow read `${{ tasks.build.message }}` across tasks. `compose.rs` is the only site naming concrete adapter types (grep over `tmx-cli/src`: the one other `tmx_adapters` import is the pure helper fn `detect_source_kind` in `commands/run.rs`, not an adapter type). Note: `docs/examples/single-file-flow.yaml` named in the evidence is a spec illustration, not a runnable fixture — it references a `./hooks/on-error.yaml` that does not ship and needs the task-20 `fetch` capability; it fails closed with a typed exit-4 resolution error, and the obligation was discharged with equivalent runnable flows instead.

- **O2 — The exit code matches the outcome — 0 on success, 1 on a failed `assert`/task, 4 on an unresolved flow — and a requested secret echoed by a task does not appear in stdout.**
  - *Claim:* `exit_code(&RunError)` maps success → 0, a failed `assert`/task → 1, and an unresolved flow → 4; a secret echoed by a task is masked out of stdout.
  - *Evidence to collect:* Read `fn exit_code(&RunError) -> i32` at the `main` seam. Run a passing flow (expect `$?` = 0), a failing-`assert` flow (expect 1), and a missing/unresolved-flow invocation (expect 4). Run a flow whose task echoes a requested secret and grep stdout — expect the secret absent.
  - *Checks:* Resolve the `exit_code` mapping so a failed assert yields 1 and an unresolved flow yields 4; trace that the final-state reporter runs the merged state through the Masker so a requested secret echoed by a task is redacted in stdout.
  - *Status:* ☑ SATISFIED — Read `exit_code`/`exit_for_status` in `main.rs` (exhaustive matches, no wildcard; the unit test proves all six categories map to distinct documented codes). Ran the real binary: passing flow → `$?` = 0; failing `assert` → 1 with valid JSON still on stdout; missing/unresolved flow → 4 with empty stdout and the search path on stderr; a `fetch` flow → 5 (capability check trips before the denying stub); unknown subcommand → 2 (clap). Secret non-leak exercised live: an env-sourced `TOKEN` echoed by an exec task — `grep supersecretvalue` found zero hits on stdout *and* stderr, and the final state shows `"leak"."message": "[REDACTED]"`. Traced the mask path: both `EngineRunFlow` and the directory tail in `commands/run.rs` run the merged state through `masker.redact_value` before it leaves the process. Negative space beyond the DoD: an *unset* secret env var fails the task (typed `secret_not_found`, run exits 1, no panic).

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — Ran independently from the main tree: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 206/206 passed (incl. the 19 tmx-cli tests driving the real binary); `scripts/purity.sh` clean. No new tunable engine limit was introduced: the scheduler bound reuses `tmx_schema::limits::CONCURRENCY_MAX`; `clock.rs`'s calendar constants (`SECONDS_PER_DAY`, `EPOCH_TO_ERA_OFFSET_DAYS`, `DAYS_PER_ERA`, …) are named units-last structural constants local to the adapter, mirroring the `model.rs` precedent — correctly not `limits` entries. Every new fn carries ≥2 meaningful assertions across its tests, negative space included (unset secret, unsupported secret source, denying stubs, `log.truncated` → no line, empty directory, unknown subcommand).

- **O4 — Run a real `flow.yaml` from the shell, pipe stdout to `jq`, and confirm the printed state, the stderr progress, and the exit code (Reviewable).**
  - *Claim:* A reviewer can run a real flow from the shell, pipe stdout to `jq`, and observe valid JSON final state, progress on stderr (not stdout), and the mapped exit code.
  - *Evidence to collect:* Run `tmx run docs/examples/single-file-flow.yaml | jq .` in the shell; observe `jq` parses stdout as one JSON object, progress text appears on stderr only, and `$?` matches the outcome.
  - *Status:* ☑ SATISFIED — Exercised from the shell against the built binary: `tmx run flow.yaml | jq '.check.passed'` printed `true`; `jq -e` validated the full state object; stderr carried the `run:`/`task:` progress lines and stdout carried nothing else; `$?` matched every outcome tried (0/1/4/5). (`docs/examples/single-file-flow.yaml` itself is not a self-contained runnable fixture — see O1's note — so equivalent real flows were used; the named example fails closed with a typed exit-4 error, which is the correct behaviour for this build.)

## Regression check

- Task 17 wires 11/15/16 into the `RunFlow` use case: trace that the Task-11 runner integration test path is unchanged — run the Task-11 core integration test (the deterministic multi-task `assert`/`exec` flow over the fakes) and confirm it still passes with the CLI composition added around it.

## Residue

The secret-non-leak check depends on the Masker (Task 09) running over final state — confirm the exercised flow actually requests and echoes a secret so the test is meaningful. Exit codes 3/5/124/130 exist in the full mapping but only 0/1/4 are in this task's DoD; do not fail the task for the others being untested here. stdout must carry only the JSON object — any stray log on stdout breaks the `| jq` contract.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED by direct evidence — the real binary was driven from the shell over all four formats, all resolution rungs, both execution paths (single file via `EngineRunFlow`; directory/folder-layout via the runner), and the full exit-code surface (0/1/2/4/5), with the secret-leak negative space verified live on both streams. The regression check holds: tmx-core's 107 tests (incl. the Task-11 runner integration path) pass unchanged, and the diff touches no core code. Residue for a later task, not a Task-17 defect: a flow whose `context`/`environment` is *reference-form* preflights green (task 15 chases references) but then fails inside the pre-existing `EngineRunFlow` re-load with a typed `unsupported_reference` exit-4 error, because the use case's `resolve_flow` does not chase references — fail-closed and out of this task's exec/assert DoD scope, but the single-file CLI path inherits it; also `docs/examples/single-file-flow.yaml` is not runnable as shipped (missing `./hooks/on-error.yaml`, needs task-20 `fetch`).
