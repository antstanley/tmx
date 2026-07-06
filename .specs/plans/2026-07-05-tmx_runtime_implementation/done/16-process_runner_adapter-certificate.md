# Done Certificate — Task 16: Process runner adapter (`exec` and `run`)

**Task:** [16-process_runner_adapter.md](16-process_runner_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

> This certificate is a verification protocol for Task 16. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 16) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Deliver `OsProcessRunner` — the first real side-effecting adapter — running `exec` (one command) and `run` (a script in a language, default `bash`) under a per-task timeout and a captured-output cap.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-05 driven-port trait `ProcessRunner` is implemented as declared, not modified; tokio stays confined to `tmx-adapters` behind a Cargo feature (Task-01 purity gate green).

## Obligations

- **O1 — `exec` and `run` execute real processes and return captured output; a non-zero exit is a typed `RunError` and the language default is `bash`.**
  - *Claim:* `exec` runs a single shell command line and `run` runs an inline `script` or a `file` path in a named language (default `bash`), both returning captured stdout/stderr; a non-zero exit yields a typed `RunError`.
  - *Evidence to collect:* Read `crates/tmx-adapters/src/process.rs` and the tokio-runtime seam in `crates/tmx-adapters/src/lib.rs`. Run the zero-exit test (expect captured output) and the non-zero-exit test (expect a typed `RunError`). Confirm the `run` language default resolves to `bash` when unspecified.
  - *Checks:* Resolve a non-zero process exit through the `From<…> for RunError` impl and confirm it yields a typed error — not a panic, and not a zero-exit `Ok`.
  - *Status:* ☑ SATISFIED — `exec_zero_exit_captures_stdout` (captured `hello`, exit 0), `exec_non_zero_exit_is_a_typed_error` (RunFailure `process_exit_nonzero`), `run_defaults_to_bash`, `run_inline_script_in_explicit_language`, `run_executes_a_script_file` all pass; `run_inner` routes non-zero through `ProcessError::NonZeroExit` → `From<ProcessError> for RunError` (process.rs:203-208, 292-321). Default `bash` confirmed in code: `DEFAULT_RUN_LANGUAGE = "bash"` used by `invocation` when `language` is unset/empty. Validator note: on macOS `/bin/sh` is bash, so the behavioural bash-ism probe cannot distinguish bash from sh on this host; the default is pinned by code reading + the pure `invocation` test.

- **O2 — A command exceeding its `timeout` is cancelled and reported, and output beyond `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large` rather than growing unbounded.**
  - *Claim:* A command over its per-task `timeout` is cancelled and reported as a typed error; captured stdout/stderr beyond `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large` instead of buffering unbounded.
  - *Evidence to collect:* Read the timeout and output-cap paths in `crates/tmx-adapters/src/process.rs`. Run the timeout test (expect cancellation + typed report) and the over-cap-output test (expect `output_too_large`). Confirm `CAPTURED_OUTPUT_MAX_BYTES` is a named units-last constant in `tmx-schema::limits`.
  - *Checks:* Trace the captured-output path so stdout/stderr beyond `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large` rather than accumulating unbounded.
  - *Status:* ☑ SATISFIED — `timeout_cancels_and_reports` passes (sleep 30 under a 50 ms budget returns promptly with `task_timeout`; child hard-killed at process.rs:170-176); `over_cap_output_is_bounded` and `under_cap_output_is_allowed` pass (`read_capped` fails the moment `buffer.len() > cap`, process.rs:233-235 — ceiling, not off-by-one). Mutation check: disabling the cap guard made `over_cap_output_is_bounded` fail, then reverted — the guard is load-bearing. `CAPTURED_OUTPUT_MAX_BYTES` is a named units-last constant in `tmx-schema::limits` (limits.rs:127, 64 MiB) and is the runner's default cap.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — validator ran all three from the main tree: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 178/178 passed, 0 skipped. No new numeric limit introduced — the one bound enforced is the existing `CAPTURED_OUTPUT_MAX_BYTES`; `DEFAULT_RUN_LANGUAGE` (an identifier default) and `READ_CHUNK_BYTES` (buffering granularity) are documented local constants, units-last named. Extra: `cargo build -p tmx-adapters --no-default-features` builds (tokio droppable) and `scripts/purity.sh` green (tokio confined to tmx-adapters).

- **O4 — Run a flow (or adapter test) exercising a successful command, a failing command, a timeout, and an over-cap output (Reviewable).**
  - *Claim:* A reviewer can run the four cases and observe captured output on success, a typed error on failure, cancellation on timeout, and `output_too_large` on over-cap.
  - *Evidence to collect:* Run the four adapter tests (success / non-zero exit / timeout / over-cap) via `cargo nextest run` — including the real-process-marked cases — and observe the four outcomes.
  - *Status:* ☑ SATISFIED — validator ran `cargo nextest run -p tmx-adapters process`: 12/12 passed, real processes executed under the default feature set (not skipped — the `process` feature is default-on). The four outcomes observed: captured stdout on success, RunFailure `process_exit_nonzero` on `exit 3`, prompt cancellation + `task_timeout` on `sleep 30` @ 50 ms, and `output_too_large` on over-cap output. Residue checked: both `run` arms (inline `-c` and file path) exercised by `run_inline_script_in_explicit_language` / `run_executes_a_script_file`; host failures funnel through `ProcessError` → `RunError`, no unwrap/expect in non-test code.

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The composition root that will consume `ProcessRunner` is Task 17.

## Residue

The over-cap and timeout tests need real-process execution (marked accordingly) — confirm they are not silently skipped in the default `nextest` run. Host/spawn failures must be typed via the `From` impl, never a panic. `run` accepts either an inline `script` or a `file` path — check both arms.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently executed evidence: `OsProcessRunner` runs `exec` (`sh -c`) and `run` (default-bash inline/file arms) as real processes, types every host failure through `ProcessError` → `RunError` (never panics), cancels on `timeout` (`task_timeout`, child hard-killed, prompt return verified), and bounds capture at `CAPTURED_OUTPUT_MAX_BYTES` (`output_too_large`; guard confirmed load-bearing by mutation, exactly-at-cap allowed). fmt/clippy/nextest all clean (178/178, 12 process tests executed, none skipped); tokio confined behind the default-on `process` feature (`--no-default-features` drops it; purity gate green); the Task-05 `ProcessRunner` port implemented as declared, unmodified. Minor non-blocking notes: on macOS the bash-default behavioural test cannot distinguish bash from sh (host quirk; default pinned by code + pure test), and a non-over-cap pump I/O error returns without killing a still-live child (rare edge, typed error either way).
