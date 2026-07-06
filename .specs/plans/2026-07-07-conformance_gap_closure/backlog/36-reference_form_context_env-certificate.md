# Done Certificate — Task 36: Reference-form context/environment execution

**Task:** [36-reference_form_context_env.md](36-reference_form_context_env.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-07 — unverified

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — reference-form context runs.** A Flow with an external-file (reference-form) `context` runs end-to-end (exit 0), with the referenced context readable by tasks — same final state as the inline equivalent.
  - *Evidence:* a test + a real `tmx run` on a `{ use: ./ctx.yaml }` context flow, exit 0, context value in state.
  - *Status:* ☐ unverified
- **O2 — reference-form environment runs.** Likewise a reference-form `environment` resolves through to execution, not fail-closed at the re-load.
  - *Status:* ☐ unverified
- **O3 — no regression, guards intact.** Inline-form flows unchanged; a dangling/cyclic reference still surfaces its typed error; resolution stays bounded (kind dispatch, depth, schema, `cyclic_flow_import`). `cargo fmt --all --check` / `clippy --all-targets --all-features -D warnings` / `nextest` (all prior + new) / `scripts/purity.sh` clean.
  - *Status:* ☐ unverified
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line (exit 0, not exit 4).
  - *Status:* ☐ unverified

## Conclusion
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
