# Done Certificate — Task 24: Secret resolver adapter (`env` / `file` / provider seam)

**Task:** [24-secret_resolver_adapter.md](24-secret_resolver_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

> This certificate is a verification protocol for Task 24. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 24) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The `SecretResolver` adapter resolves `env` and `file` sources with a provider trait seam, wired so resolved secrets register with the Masker end to end.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Wiring the `SecretResolver` into the Task 17 composition root (`crates/tmx-cli/src/compose.rs`) must not change the existing `exec`/`assert` run path.

## Obligations

- **O1 — An `env` and a `file` secret resolve for a task that lists them, and the value is redacted everywhere it would be emitted end to end via the CLI.**
  - *Claim:* an `env` secret (a host env var) and a `file` secret (a path) resolve for a task that lists them, and the resolved value is redacted everywhere it would be emitted end to end through the CLI.
  - *Evidence to collect:* read `crates/tmx-adapters/src/secret.rs`; run the tests confirming an `env` and a `file` secret resolve and are masked in output; read the compose wiring in `crates/tmx-cli/src/compose.rs` registering each resolved value with the Masker.
  - *Checks:* trace that only names in a task's `secrets` array reach the resolver, and each resolved value is registered with the Masker before any output.
  - *Status:* ☑ SATISFIED — `resolve_secrets` (runner.rs:875-914) iterates only `task.secrets` and registers each value with the Masker before push/insert; CLI tests `a_requested_secret_echoed_by_a_task_never_reaches_stdout` (env) and `a_file_secret_echoed_by_a_task_never_reaches_stdout` (file) pass, asserting the raw value absent from stdout AND stderr and `[REDACTED]` in the final state (genuine assertions, not substring no-ops). Independently exercised: env-secret flow run live, value `super-sensitive-review-token` appeared 0 times in stdout/stderr, state showed `[REDACTED]`.

- **O2 — An unrequested secret name is never resolved or bound into a task's scope, and the provider seam compiles without a concrete backend (negative space).**
  - *Claim:* a secret name a task did not list is never resolved or bound into that task's scope; the provider trait seam (`aws-sm`/`vault`/… future adapters) compiles with no concrete backend.
  - *Evidence to collect:* run the test asserting an unrequested secret name is never resolved or bound into scope; read `crates/tmx-adapters/src/secret.rs` for the provider trait seam and confirm `cargo build` compiles it with no concrete provider adapter present.
  - *Checks:* trace that resolution is keyed only off the task's declared `secrets` set — an unrequested name has no resolution path into scope.
  - *Status:* ☑ SATISFIED — CLI test `an_unrequested_secret_is_never_resolved_or_bound` passes: a context secret with a deliberately unreadable `file` source is never touched when no task lists it (flow exits 0), proving no resolution path exists; the value is absent from scope. The `SecretProvider` trait seam compiles with zero concrete backends (`BuiltinSecretResolver::new()` wired in compose.rs; `cargo build`/clippy clean), and `a_provider_source_with_no_registered_backend_is_unavailable` proves the seam is reachable and typed (`secret_provider_unavailable`), never silently empty.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm any new bound is a named units-last constant in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☑ SATISFIED — verifier ran independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 279/279 passed. The one new bound, `SECRET_FILE_MAX_BYTES` (limits.rs:196), is a named units-last constant with a compile-time assert. `scripts/purity.sh` green.

- **O4 — Run a flow whose task requests an `env` secret and echoes it, and confirm the value is masked in stdout and the run store (Reviewable).**
  - *Claim:* a reviewer can run a flow whose task requests an `env` secret and echoes it and observe the value masked in stdout and in the run store record.
  - *Evidence to collect:* run the reviewable flow whose task requests an `env` secret and echoes it; observe the value masked in stdout and in the run store record.
  - *Status:* ☑ SATISFIED (run-store half deferred per Residue) — verifier ran the flow live: task echoes `${{ secrets.TOKEN }}` from env var `TMX_REVIEW_SECRET`; exit 0, stdout state shows `"message": "[REDACTED]"`, raw value appears 0 times in stdout and stderr. The run store (Task 27) is not yet built (not wired in compose.rs, not in done/), so its half is deferred exactly as the Residue directs.

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☑ PRESERVED — compose.rs swaps `EnvSecretResolver` for `BuiltinSecretResolver` only (same port, same call sites); all pre-existing CLI run tests (exec/assert path, env-secret masking) pass in the 279/279 suite.

## Residue

- The run-store half of O4 depends on Task 27 (run store) being present; if it is not yet built when this task is validated, exercise the stdout masking and note the run-store check as deferred.
- Step 2's defensive property — a secret is never accepted as a plain CLI flag (a process-table leak); ad-hoc injection sets a host env var referenced by `secretSource: { env: … }` — is outside the four DoD items but worth a spot-check: confirm no CLI flag path accepts a raw secret value.
- Masking correctness depends on the Task 09 Masker (short-value scan floor, nested-JSON redaction); confirm the masking assertion is genuine, not a substring no-op.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED and the regression check PRESERVED. The `BuiltinSecretResolver` resolves `env`/`file` sources and dispatches `provider` sources through an open trait seam; every failure path (unset/empty env, missing/oversized/non-UTF-8/empty file, unknown provider, empty provider value, empty literal) is a typed `RunError`, verified by 28 passing secret tests plus live CLI runs. The attempt-3 empty-secret fix closes the exit-101 panic at a single runner choke point (runner.rs:901) covering literal/provider/future paths, with adapter-level defence in depth; all three empty-secret shapes exercised live exit 1, never 101. Residue spot-checks pass: no CLI flag accepts a raw secret value (args.rs has no secret surface), masking assertions are genuine (raw value asserted absent from both streams). fmt/clippy/nextest (279/279)/purity all green; O4's run-store half deferred per Residue (Task 27 unbuilt).
