# Done Certificate — Task 25: Environment providers and the ephemeral lifecycle

**Task:** [25-environment_providers.md](25-environment_providers.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 25. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 25) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship the `EnvironmentProvider` port with `BinaryProvider` and `FlowProvider` adapters, the provider manifest types, and the ephemeral deploy/clean/destroy lifecycle wrapping `RunFlow`, driven by `tmx env`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-11 `PipelineRunner` and Task-12 `HookRunner` still run a plain flow unchanged — a `FlowProvider` method recurses into that same runner under the `FLOW_DEPTH_MAX` bound, not a second engine; Task-15 preflight still fails fast before any task executes; Task-16 `OsProcessRunner` still executes `exec`/`run`; Task-17 `tmx run` still runs a provider-less flow.

## Obligations

- **O1 — `tmx env` drives a BinaryProvider method and a FlowProvider method, and `tmx run` provisions/cleans a standing environment per the flags.**
  - *Claim:* `tmx env` maps provider methods 1:1 plus `up`/`down` aggregates; a `BinaryProvider` method invokes the manifest binary with the method subcommand (passing the resolved environment/options, the process result being the method result); a `FlowProvider` method runs its inline tasks / referenced Flow as a Flow; `tmx run` runs `deploy → run → clean` by default and honours `--keep`, `--no-deploy`, `--local`.
  - *Evidence to collect:* read `crates/tmx-cli/src/commands/env.rs` for the method→subcommand mapping and the `up`/`down` aggregates; read `crates/tmx-adapters/src/provider/` for the `BinaryProvider` and `FlowProvider` adapters and `crates/tmx-schema/src/provider.rs` for the manifest types; run the named tests exercising a `BinaryProvider` method (binary invocation) and a `FlowProvider` method (body run as a Flow); run `tmx run` once per `--keep`/`--no-deploy`/`--local` and observe that deploy/clean occur or are skipped per flag.
  - *Checks:* resolve a `FlowProvider` method to a recursion into the same `PipelineRunner` (inheriting the recursion depth bound), not a separate engine.
  - *Status:* ☐ unverified

- **O2 — A failed method is EnvironmentError (exit 5) not RunFailure, teardown still runs after a failure, and an out-of-schema options block is rejected at preflight.**
  - *Claim:* a failed provider method is an `EnvironmentError` (exit 5), distinct from a pipeline `RunFailure` (exit 1); `clean`/`destroy` run best-effort even after a failed or cancelled run, with the context `destroy` hook still firing; an `environment.options` block violating the manifest's `optionsSchema` is rejected at preflight.
  - *Evidence to collect:* run the test that forces a provider method to fail and observe exit 5, then trace that `clean`/`destroy` still execute afterwards; run the negative test seeding an `options` block that violates `optionsSchema` and confirm preflight rejects it before any task runs.
  - *Checks:* confirm a failed method maps to `EnvironmentError` (exit 5), distinct from `RunFailure` (exit 1).
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Reviewable: `tmx env deploy`/`clean` against a binary and a flow provider, with the exit code and best-effort teardown on a forced failure.**
  - *Claim:* a reviewer can run `tmx env deploy` then `tmx env clean` against a binary provider and a flow provider and observe the mapped exit code, and on a forced method failure observe exit 5 with `clean`/`destroy` still firing.
  - *Evidence to collect:* run `tmx env deploy` then `tmx env clean` against a binary-provider fixture and a flow-provider fixture; force a method failure and confirm exit 5 and that best-effort teardown still runs.
  - *Status:* ☐ unverified

## Regression check

- Task-11/12 runner + Task-15 preflight: trace a plain `tmx run flow.yaml` with no `environment.provider` — it still preflights and runs through the `PipelineRunner` and exits 0, i.e. the provider lifecycle wrapper is a no-op when absent.

## Residue

- The `--local` path (use a standing environment, skip provisioning) and the interaction of `--keep` with a failed run are edge cases worth a trace beyond the happy path.
- Best-effort `destroy` overlaps Task 29 (cancelled-run teardown); confirm the two paths do not double-fire the context `destroy` hook.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
