# Done Certificate — Task 29: Cancellation, timeout, and interrupt

**Task:** [29-cancellation_and_timeouts.md](29-cancellation_and_timeouts.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

- **O2 — An adapter that ignores the grace period is hard-stopped, and best-effort teardown still runs.**
  - *Claim:* an in-flight adapter that does not return within the grace period is hard-stopped at the deadline — no cancelled run is held hostage — and `clean`/`destroy` (the lifecycle `finally` and the provider methods) still run best-effort after the forced stop.
  - *Evidence to collect:* run the test using a stub adapter that ignores cancellation (never returns during the grace window) and confirm it is hard-stopped at the grace deadline; confirm best-effort `clean`/`destroy` teardown still runs after the hard stop.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm `CANCEL_GRACE_MS` is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Reviewable: a long flow with a short `--timeout` and a separate SIGINT, confirming the grace behaviour, `destroy` firing, and the exit codes.**
  - *Claim:* a reviewer can run a long-running flow with a short `--timeout` and observe exit 124 with `destroy` fired within the grace period, then run the same flow and send SIGINT and observe exit 130 with `destroy` fired.
  - *Evidence to collect:* run a long flow (e.g. a sleep well beyond the timeout) with a short `--timeout` and observe the grace window elapse, `destroy` fire, and exit 124; separately run the flow and send SIGINT and observe `destroy` fire and exit 130.
  - *Status:* ☐ unverified

## Regression check

- Task-16/20/21/22/23 adapters: trace that a fast flow with no `--timeout` and no interrupt still completes through the token-threaded adapters and exits 0 — the always-present token is a no-op when never triggered.

## Residue

- `--grace` override and the `CANCEL_GRACE_MS` default interact; confirm a `--grace 0` forces an immediate hard stop and a large `--grace` is still bounded.
- Best-effort teardown here overlaps Task 25 provider `clean`/`destroy`; confirm a SIGINT arriving during teardown itself (double interrupt) does not wedge the run or double-fire `destroy`.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
