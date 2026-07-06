# Done Certificate — Task 36: Reference-form context/environment execution

**Task:** [36-reference_form_context_env.md](36-reference_form_context_env.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-07

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — reference-form context runs.** A Flow with an external-file (reference-form) `context` runs end-to-end (exit 0), with the referenced context readable by tasks — same final state as the inline equivalent.
  - *Evidence:* a test + a real `tmx run` on a `{ use: ./ctx.yaml }` context flow, exit 0, context value in state.
  - *Status:* ☑ SATISFIED — core test `engine_run_flow_reference_form_context_runs_end_to_end_like_the_inline_form` drives `EngineRunFlow` (the fixed library path) to `RunStatus::Ok` with the same final state as the inline equivalent; `engine_run_flow_reference_form_context_holds_the_resolved_value` proves the referenced value is actually inlined and read (wrong expectation fails). CLI test `a_reference_form_context_runs_end_to_end_and_a_task_reads_its_value` and my own manual `tmx run` on `context: ./context.yaml` both exit 0 with `read.passed=true` (value `hello-from-ref` read from the referenced context).
- **O2 — reference-form environment runs.** Likewise a reference-form `environment` resolves through to execution, not fail-closed at the re-load.
  - *Status:* ☑ SATISFIED — `engine_run_flow_reference_form_environment_runs_end_to_end` (core) and `a_reference_form_environment_runs_end_to_end` (CLI) both resolve a provider-less `local` referenced environment through to `Ok` / exit 0.
- **O3 — no regression, guards intact.** Inline-form flows unchanged; a dangling/cyclic reference still surfaces its typed error; resolution stays bounded (kind dispatch, depth, schema, `cyclic_flow_import`). `cargo fmt --all --check` / `clippy --all-targets --all-features -D warnings` / `nextest` (all prior + new) / `scripts/purity.sh` clean.
  - *Status:* ☑ SATISFIED — `resolve_referenced_flow` reuses the identical private `resolve_references` step preflight runs, so kind dispatch / `JSON_DEPTH_MAX` / schema / `reference_kind_mismatch` bounds are unchanged; an inline-form flow carries no reference so it resolves as before (no double resolution). Dangling reference is a typed `Resolution`/`reference_not_found` error in core (`engine_run_flow_dangling_context_reference_is_a_typed_resolution_error`), CLI (exit 4), and my manual run. Verified clean: `cargo fmt --all --check` (exit 0), `clippy --all-targets --all-features -D warnings` (Finished, no warnings), `cargo nextest run` (475 passed, 0 failed), `scripts/purity.sh` (green).
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line (exit 0, not exit 4).
  - *Status:* ☑ SATISFIED — I ran the real `target/debug/tmx run` on a flow with `context: ./context.yaml`: the `read` assert read `env.GREETING` from the referenced context and the run exited 0; a dangling `context: ./nope.yaml` exited 4 with a typed `reference_not_found` on stderr and no JSON on stdout.

## Conclusion
VERDICT: DONE
CONFIDENCE: high
SUMMARY: The library `EngineRunFlow::run` path now routes load→resolve through the new `resolve_referenced_flow`, which applies preflight's exact `resolve_references` inlining before `resolve`, closing the exit-4 fail-close on reference-form `context`/`environment` without introducing new bounds or regressing the inline path. All four obligations discharged with run/observed evidence; full suite (475) + fmt + clippy + purity green; Reviewable independently exercised on the real binary.
