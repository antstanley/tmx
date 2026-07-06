# Task 17 — `tmx run` (the first end-to-end path)

**Plan:** [plan.md](../plan.md) · **Certificate:** [17-cli_run_minimal-certificate.md](17-cli_run_minimal-certificate.md)

**Implements:** [07-cli.md](../../../07-cli.md) §Command → use case mapping (`run`), §`tmx run` (flow resolution), §stdout / stderr contract (final-state), §Exit codes; [02-crate-architecture.md](../../../02-crate-architecture.md) §Composition root
**Depends on:** 11, 15, 16
**Produces:** the `tmx` binary running `tmx run flow.yaml` end to end — load, preflight, execute `exec`/`assert` tasks, print masked final state to stdout, and return the mapped exit code
**Pointers:** `crates/tmx-cli/src/main.rs` (new), `crates/tmx-cli/src/args.rs` (new), `crates/tmx-cli/src/compose.rs` (new), `crates/tmx-cli/src/config.rs` (new), `crates/tmx-cli/src/commands/run.rs` (new); the always-on adapters `crates/tmx-adapters/src/clock.rs` + `idgen.rs` (new) and the minimal serial `scheduler.rs` / `env` `secret.rs` seams (extended by tasks 18/24)

## Steps

- [x] Define the `run` command and its core arguments with `clap`, and implement the flow-resolution order (`--file/-f` → positional → `$TMX_FLOW` → `./flow.{…}`/`./tmx.{…}` → folder-layout → `ResolutionError` printing the search path).
- [x] Build the always-on infrastructure adapters this task owns: `SystemClock` (`clock.rs`) and `Uuidv7Generator` (`idgen.rs`) for per-task timing and the run id, a minimal serial production `Scheduler` sufficient for the default `concurrency: 1` (the bounded `TokioScheduler` arrives in task 18), and a minimal `env` `SecretResolver` so a requested secret resolves and the masking guarantee is demonstrable end to end (task 24 extends it to `file` and the provider seam).
- [x] Build the composition root wiring those plus the loader, resolver, validator, and process runner into the `RunFlow` use case; keep this the only place concrete adapter types are named, with the not-yet-built executor adapters (`fetch`/`file`/`store`/`chat`) present as denying stubs the capability check reports.
- [x] Implement the final-state stdout reporter (the merged JSON object, masked) so `tmx run flow.yaml | jq` works with no flag, and route human progress to stderr.
- [x] Implement the single `fn exit_code(&RunError) -> i32` mapping every `ErrorCategory` to its documented code (0/1/3/4/5/124/130), used only at the `main` seam.

## Definition of done

- [x] `tmx run flow.yaml` (any of the four formats) loads, preflights, executes a flow of `exec` and `assert` tasks, and prints the masked final Pipeline state as one JSON object to stdout.
- [x] The exit code matches the outcome — 0 on success, 1 on a failed `assert`/task, 4 on an unresolved flow — and a requested secret echoed by a task does not appear in stdout (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run a real `flow.yaml` from the shell, pipe stdout to `jq`, and confirm the printed state, the stderr progress, and the exit code.
