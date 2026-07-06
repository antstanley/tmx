# Done Certificate — Task 37: Adapter/scorer fidelity

**Task:** [37-adapter_scorer_fidelity.md](37-adapter_scorer_fidelity.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-07 — unverified

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — store timeout.** A `store` task honours its per-task `timeout`, surfacing typed `task_timeout` on breach under the same cancellation contract as `exec`/`run`/`fetch`.
  - *Evidence:* a test + a real `tmx run` where a `store` task against a slow/unreachable endpoint times out typed at ~its timeout.
  - *Status:* ☐ unverified
- **O2 — llmRubric endpoint.** The `llmRubric` scorer's `apiUrl`/`apiKey` route the judge call to the configured endpoint; absent them, the composed default is used.
  - *Evidence:* an `apiUrl` pointed at a local server is observed to receive the judge request; the absent-field case still uses the default.
  - *Status:* ☐ unverified
- **O3 — no regression.** Existing store/chat/eval tests stay green; `cargo fmt --all --check` / `clippy --all-targets --all-features -D warnings` / `nextest` (all prior + new) / `scripts/purity.sh` clean; no new hard-coded bound.
  - *Status:* ☐ unverified
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line.
  - *Status:* ☐ unverified

## Conclusion
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
