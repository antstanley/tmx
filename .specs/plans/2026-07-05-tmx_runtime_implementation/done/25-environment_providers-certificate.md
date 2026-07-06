# Done Certificate — Task 25: Environment providers and the ephemeral lifecycle

**Task:** [25-environment_providers.md](25-environment_providers.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — all obligations discharged by an independent verifier

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
  - *Status:* ☑ SATISFIED — `crates/tmx-cli/src/commands/env.rs` maps `bootstrap`/`deploy`/`clean`/`destroy` 1:1 via `plan()` plus `up` = bootstrap→deploy (stop-on-fail) and `down` = clean→destroy (best-effort); `FlowProvider::invoke` (`crates/tmx-adapters/src/provider/mod.rs`) calls `PipelineRunner::new(config).run(…, 0)` — the same engine, so nested `flow` tasks inherit `FLOW_DEPTH_MAX`; `BinaryProvider::invoke` builds `<binary> <subcommand>` with the serialised `Environment` on stdin and the process stdout as the method result. Exercised live: `tmx env deploy`/`clean` against a flow provider (marker log `DC`, exit 0) and a binary provider (stdout JSON `{"deployed":true}` surfaced as the method result, exit 0); `tmx env up` logged `BD`, `down` logged `CX`. `tmx run` observed: default → `DC`, `--keep` → `D`, `--no-deploy` → ``, `--local` → `` (main run's state on stdout in all four). Integration tests `tests/cli_env.rs` pin all of the above.

- **O2 — A failed method is EnvironmentError (exit 5) not RunFailure, teardown still runs after a failure, and an out-of-schema options block is rejected at preflight.**
  - *Claim:* a failed provider method is an `EnvironmentError` (exit 5), distinct from a pipeline `RunFailure` (exit 1); `clean`/`destroy` run best-effort even after a failed or cancelled run, with the context `destroy` hook still firing; an `environment.options` block violating the manifest's `optionsSchema` is rejected at preflight.
  - *Evidence to collect:* run the test that forces a provider method to fail and observe exit 5, then trace that `clean`/`destroy` still execute afterwards; run the negative test seeding an `options` block that violates `optionsSchema` and confirm preflight rejects it before any task runs.
  - *Checks:* confirm a failed method maps to `EnvironmentError` (exit 5), distinct from `RunFailure` (exit 1).
  - *Status:* ☑ SATISFIED — forced failures observed live: a binary method exiting 7 → `Environment [provider_method_failed]`, exit 5; a flow method whose task exits 1 → exit 5 (the `RunFailure` is re-categorised in `method_failed`/`BinaryProvider::invoke` and the non-`ok` pipeline branch of `FlowProvider::invoke`). Teardown after failure: `tmx run` with a failing `assert` → exit 1 with marker log `DC` (clean still ran); `tmx run` with a failing deploy → clean still ran (`C`), pipeline never started, exit 5; `tmx env down` with a failing clean → destroy still ran (`X`), exit 5. Preflight gate: `options: {region: eu}` against `optionsSchema` requiring `cluster` → `Validation [provider_options_invalid]`, exit 3, deploy marker never written — for both `tmx env deploy` and `tmx run`. Unit negatives in `provider/mod.rs` and integration tests `a_forced_flow_provider_method_failure_is_exit_five_not_a_run_failure`, `a_forced_binary_provider_method_failure_is_exit_five`, `out_of_schema_options_are_rejected_before_any_method_runs`, `tmx_run_teardown_still_runs_after_a_failed_run` pin these.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — independently run 2026-07-06: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 301/301 passed; `scripts/purity.sh` green. No new numeric bound introduced (recursion reuses `FLOW_DEPTH_MAX`), so no new `limits` constant was required.

- **O4 — Reviewable: `tmx env deploy`/`clean` against a binary and a flow provider, with the exit code and best-effort teardown on a forced failure.**
  - *Claim:* a reviewer can run `tmx env deploy` then `tmx env clean` against a binary provider and a flow provider and observe the mapped exit code, and on a forced method failure observe exit 5 with `clean`/`destroy` still firing.
  - *Evidence to collect:* run `tmx env deploy` then `tmx env clean` against a binary-provider fixture and a flow-provider fixture; force a method failure and confirm exit 5 and that best-effort teardown still runs.
  - *Status:* ☑ SATISFIED — performed live from the shell against scratch fixtures with the compiled `target/debug/tmx`: flow provider `deploy` then `clean` → exit 0/0, marker log `DC`; binary provider `deploy` then `clean` → exit 0/0 with the script's stdout JSON as each method result; forced binary `deploy` failure → exit 5; `env down` with a failing `clean` → `destroy` still fired, exit 5.

## Regression check

- Task-11/12 runner + Task-15 preflight: trace a plain `tmx run flow.yaml` with no `environment.provider` — it still preflights and runs through the `PipelineRunner` and exits 0, i.e. the provider lifecycle wrapper is a no-op when absent.
  - ☑ VERIFIED — `a_flow_without_a_provider_is_unaffected_by_the_lifecycle` passes, and the wrapper is a guarded no-op by construction (`provider_loaded` is `None` when `environment`/`provider` is absent or `--local`). Residue traced: `--local`/`--no-deploy` observed skipping the lifecycle live; `--keep` after a failed deploy skips the clean and surfaces exit 5; the context `destroy` hook fires inside the pipeline while provider `clean` is a separate method invocation — the failed-run trace logged exactly `DC`, no double-fire. `tmx env` on a provider-less flow is a typed `no_environment` Environment error (exit 5), not a panic. Full suite green (301/301) confirms no regression in Tasks 11/12/15/16/17 surfaces.

## Residue

- The `--local` path (use a standing environment, skip provisioning) and the interaction of `--keep` with a failed run are edge cases worth a trace beyond the happy path.
- Best-effort `destroy` overlaps Task 29 (cancelled-run teardown); confirm the two paths do not double-fire the context `destroy` hook.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently produced evidence — gates run clean (fmt/clippy/nextest 301/301/purity), and the whole lifecycle surface was exercised live from the shell: `tmx env` methods 1:1 plus `up`/`down` against both adapter types, `FlowProvider` recursing into the shared `PipelineRunner`, `BinaryProvider` subcommand invocation with the environment on stdin, exit-5 environment errors distinct from exit-1 run failures, best-effort teardown after failed deploy/run/clean, and the optionsSchema preflight gate rejecting out-of-schema options (exit 3) before any method runs. The provider wrapper is a proven no-op for a provider-less flow.
