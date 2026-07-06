# Task 16 — Process runner adapter (`exec` and `run`)

**Plan:** [plan.md](../plan.md) · **Certificate:** [16-process_runner_adapter-certificate.md](16-process_runner_adapter-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Executor ports (`ProcessRunner`), §exec vs run; [04-execution-engine.md](../../../04-execution-engine.md) §Limits (`CAPTURED_OUTPUT_MAX_BYTES`)
**Depends on:** 05
**Produces:** `OsProcessRunner` — the first real side-effecting adapter — running `exec` (one command) and `run` (a script in a language, default `bash`) under a per-task timeout and a captured-output cap
**Pointers:** `crates/tmx-adapters/src/process.rs` (new), `crates/tmx-adapters/src/lib.rs` (the tokio-runtime seam)

## Steps

- [x] Implement `exec` (a single shell command line) and `run` (an inline `script` or a `file` path in a named language/interpreter, defaulting to `bash`) behind the `ProcessRunner` port.
- [x] Enforce the per-task `timeout` and bound captured stdout/stderr by `CAPTURED_OUTPUT_MAX_BYTES` (`output_too_large` on overflow); translate a spawn/exit failure into a typed `RunError` via a `From` impl — never panic on a host failure.
- [x] Put the adapter behind a Cargo feature and keep tokio confined to this crate; expose the async port method the runner awaits.
- [x] Add tests for a zero-exit command, a non-zero exit, a timeout, and an over-cap output (marked for real-process execution where needed).

## Definition of done

- [x] `exec` and `run` execute real processes and return captured output; a non-zero exit is a typed `RunError` and the language default is `bash`.
- [x] A command exceeding its `timeout` is cancelled and reported, and output beyond `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large` rather than growing unbounded (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run a flow (or adapter test) exercising a successful command, a failing command, a timeout, and an over-cap output.
