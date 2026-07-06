# Done Certificate — Task 29: Cancellation, timeout, and interrupt

**Task:** [29-cancellation_and_timeouts.md](29-cancellation_and_timeouts.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

> This certificate is a verification protocol for Task 29. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 29) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Thread a cancellation token from the root into every adapter call, so `--timeout` and SIGINT stop in-flight work within a grace period, fire `destroy`, and exit 124/130.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-16 process, Task-20 http, Task-21 fs, Task-22 store, and Task-23 chat adapters still complete a normal call — the token is awaited alongside the work and is a no-op when never triggered; the Task-12 `destroy` hook still fires on every terminal status; Task-17 exit-code mapping still returns 0 on success.

## Obligations

- **O1 — A `--timeout`-exceeded run and a SIGINT'd run both stop dispatching, give in-flight work the grace period, fire `destroy`, and exit 124/130 respectively.**
  - *Claim:* on `--timeout` (via the `Clock`) or SIGINT the `Scheduler` stops dispatching new work, in-flight adapters get a grace period (`CANCEL_GRACE_MS`, default 5000 ms; `--grace` overrides), the run ends `timed_out`/`cancelled`, the `destroy` hook fires, and `main` maps the outcome to exit 124 (timeout) / 130 (interrupt).
  - *Evidence to collect:* read `crates/tmx-core/src/runner.rs` for the token threaded from the root and the `Scheduler` ceasing dispatch on cancel; read `crates/tmx-adapters/src/{process,http,fs,store,chat}.rs` for each adapter awaiting the work alongside the token; read `crates/tmx-cli/src/main.rs` for the SIGINT handler, the `--timeout` trigger, and the 124/130 exit mapping; run the named tests for a `--timeout`-exceeded run (ends `timed_out` → exit 124) and a SIGINT'd run (ends `cancelled` → exit 130), both stopping dispatch and firing `destroy`.
  - *Checks:* trace the cancellation token from the root into each adapter await and confirm the grace-then-hard-stop ordering.
  - *Status:* ☑ SATISFIED — token threaded root→`Ports.cancel` (`compose.rs::ports()` → `runner.rs::Ports`); the sequential runner reads `requested_reason()` at the top of its loop and stops dispatching (`runner.rs` execute loop), and every adapter call is awaited inside `ports.cancel.guard(run_step(...))` — one uniform seam directly above every adapter await, equivalent to per-adapter threading. Watchers in `run.rs::spawn_cancellation_watchers`/`escalate` do request → grace (`resolve_grace_ms`, default `CANCEL_GRACE_MS`) → hard. Exercised live: `tmx run flow.yaml --timeout 2s --grace 1s` ended `timed_out` in ~3 s (timeout + grace observed), fired `destroy` (DESTROY_FIRED written), never dispatched the second task, exit 124; a real SIGINT to a running flow ended `cancelled`, fired `destroy`, exit 130. Integration tests `cancellation.rs` (hard-cancelled in-flight task → `timed_out` + destroy + no second dispatch; pre-requested cancel → clean stop, `cancelled`) pass.

- **O2 — An adapter that ignores the grace period is hard-stopped, and best-effort teardown still runs.**
  - *Claim:* an in-flight adapter that does not return within the grace period is hard-stopped at the deadline — no cancelled run is held hostage — and `clean`/`destroy` (the lifecycle `finally` and the provider methods) still run best-effort after the forced stop.
  - *Evidence to collect:* run the test using a stub adapter that ignores cancellation (never returns during the grace window) and confirm it is hard-stopped at the grace deadline; confirm best-effort `clean`/`destroy` teardown still runs after the hard stop.
  - *Status:* ☑ SATISFIED — `cancellation.rs::a_hard_cancelled_in_flight_task_...` uses `HangingProcessRunner` (a `std::future::pending()` adapter) and the guard drops it at hard cancel, run resolves `timed_out` with `destroy` fired (test passes); `cancel.rs::guard_hard_stops_pending_work_...` proves the drop + waker wake at the unit level. The process side verified live: a `--timeout`-cancelled `sleep 63` child was actually killed (`kill_on_drop(true)`, `pgrep` empty after exit). Teardown runs through a fresh never-triggered token (`runner.rs` teardown_ports struct-update; CLI `teardown_ports()`/`invoke_teardown`), so the finally is not insta-cancelled by the run's own hard-cancelled token.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm `CANCEL_GRACE_MS` is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — independently run 2026-07-06: `cargo fmt --all --check` clean, `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo nextest run` 368/368 passed, `scripts/purity.sh` green (the token is pure std, no tokio edge into tmx-core). `CANCEL_GRACE_MS` reused from `tmx-schema::limits` (units-last, pre-existing); the duration factors in `config.rs` are local named consts (`MILLISECONDS_PER_*`). Negative-space tests present (garbage/empty duration, bare `--timeout` flag, pre-fired cancel).

- **O4 — Reviewable: a long flow with a short `--timeout` and a separate SIGINT, confirming the grace behaviour, `destroy` firing, and the exit codes.**
  - *Claim:* a reviewer can run a long-running flow with a short `--timeout` and observe exit 124 with `destroy` fired within the grace period, then run the same flow and send SIGINT and observe exit 130 with `destroy` fired.
  - *Evidence to collect:* run a long flow (e.g. a sleep well beyond the timeout) with a short `--timeout` and observe the grace window elapse, `destroy` fire, and exit 124; separately run the flow and send SIGINT and observe `destroy` fire and exit 130.
  - *Status:* ☑ SATISFIED — exercised for real by the validator (2026-07-06) with a `sleep 60` flow + a `destroy` hook writing a marker file: (a) `tmx run flow.yaml --timeout 2s --grace 1s --no-store` → `task: error slow: the run exceeded its --timeout budget`, `hook: ok destroy`, `run: timed_out in 2974ms` (timeout+grace), DESTROY_FIRED marker written, second task never started, **exit 124**; (b) same flow, real `kill -INT` after 2 s → `run: cancelled`, destroy fired, **exit 130**; (c) `--grace 0` → immediate hard stop (~1 s total), exit 124.

## Regression check

- Task-16/20/21/22/23 adapters: trace that a fast flow with no `--timeout` and no interrupt still completes through the token-threaded adapters and exits 0 — the always-present token is a no-op when never triggered.
- **Result: PASS** — live `tmx run fast.yaml --no-store` exits 0; `cancellation.rs::a_never_triggered_token_runs_the_flow_to_completion_unaffected` (both tasks dispatched, status `ok`) and the full 368-test suite (including all pre-existing runner/hooks/produces/adapter tests, updated only to add the `cancel` field) pass. Per-task adapter timeouts stay `ErrorCategory::RunFailure` (`task_timeout`), so `cancel_reason_of` (category `Timeout`/`Interrupt` only, produced solely by `CancelReason::to_error`) cannot misclassify a task failure as a run cancellation — verified by grep over all non-test producers.

## Residue

- `--grace` override and the `CANCEL_GRACE_MS` default interact; confirm a `--grace 0` forces an immediate hard stop and a large `--grace` is still bounded.
  — *Discharged:* `--grace 0` verified live (immediate hard stop, exit 124 in ~1 s) and unit-tested (`resolve_grace_ms(Some("0")) == 0`). A large `--grace` is bounded by the user's own value (no cap is specified by the spec); the run is never hostage because the hard stop always follows.
- Best-effort teardown here overlaps Task 25 provider `clean`/`destroy`; confirm a SIGINT arriving during teardown itself (double interrupt) does not wedge the run or double-fire `destroy`.
  — *Discharged by design + observation:* teardown runs through a fresh never-triggered token the watchers never touch, and `destroy` fired exactly once in every observed run; a second SIGINT is absorbed by the installed tokio handler (it cannot re-trigger `escalate`, whose `ctrl_c()` is awaited once). Note the flip side: teardown itself is deliberately uncancellable.
- *Validator note (non-blocking):* an unparseable `--timeout` value (e.g. `--timeout bogus` or `--timeout 1.5s`) is silently ignored — the run proceeds with no timeout (`run.rs`: `args.timeout.as_deref().and_then(parse_duration_ms)`), whereas an unparseable `--grace` falls back to the default. Rejecting it as a usage error would be safer; consider tightening in a follow-up.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently produced evidence — fmt/clippy/nextest (368/368) and purity green; the two-phase token (request → grace → hard stop) is threaded root-to-adapter and exercised live: `--timeout` ends the run `timed_out`/exit 124 with the grace window observed, a real SIGINT ends it `cancelled`/exit 130, `destroy` fires as the finally in both, the second task is never dispatched, a grace-ignoring hanging adapter is hard-stopped (future dropped, real child killed via `kill_on_drop`), and a never-triggered token leaves a fast flow at exit 0. One non-blocking note recorded in Residue (silently ignored unparseable `--timeout`).
