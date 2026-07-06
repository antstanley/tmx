# Task 29 — Cancellation, timeout, and interrupt

**Plan:** [plan.md](../plan.md) · **Certificate:** [29-cancellation_and_timeouts-certificate.md](29-cancellation_and_timeouts-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Concurrency, cancellation, timeouts; [08-errors-and-observability.md](../../../08-errors-and-observability.md) §Cancellation, timeout, interrupt
**Depends on:** 12, 16, 20, 21, 22, 23
**Produces:** a cancellation token threaded from the root into every adapter call, so `--timeout` and SIGINT stop in-flight work within a grace period, fire `destroy`, and exit 124/130
**Pointers:** `crates/tmx-core/src/runner.rs` (token threading), `crates/tmx-adapters/src/{process,http,fs,store,chat}.rs` (await alongside the token), `crates/tmx-cli/src/main.rs` (signal + exit)

## Steps

- [x] Thread a cancellation token from the root into every adapter call, awaited alongside the work; trigger it on `--timeout` (via the `Clock`) and on SIGINT.
- [x] On cancel: the `Scheduler` stops dispatching new work, in-flight adapters get a grace period (`CANCEL_GRACE_MS`, default 5000 ms; `--grace` overrides) then a hard stop.
- [x] Fire the `destroy` hook (the `finally` of the lifecycle) and run `clean`/`destroy` provider methods best-effort even after a cancelled run.
- [x] End the run `cancelled`/`timed_out` and map to exit 124 (timeout) / 130 (interrupt) at the `main` seam.

## Definition of done

- [x] A `--timeout`-exceeded run and a SIGINT'd run both stop dispatching, give in-flight work the grace period, fire `destroy`, and exit 124/130 respectively.
- [x] An adapter that ignores the grace period is hard-stopped (negative space: no cancelled run is held hostage), and best-effort teardown still runs.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run a long flow with a short `--timeout` and a separate SIGINT, and confirm the grace behaviour, the `destroy` firing, and the exit codes.
