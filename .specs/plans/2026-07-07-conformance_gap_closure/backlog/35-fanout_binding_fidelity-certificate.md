# Done Certificate — Task 35: Fan-out binding fidelity

**Task:** [35-fanout_binding_fidelity.md](35-fanout_binding_fidelity.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-07 — unverified

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — `as:` alias honoured.** A `map` declaring `as: <name>` binds the element under `<name>`; `${{ <name>.* }}` resolves through the interpolator for object, array, and scalar elements. With no `as:`, the element binds under `item` (default preserved).
  - *Evidence:* tests + a real `tmx run` where `as: region` reads `${{ region }}`; the no-alias default reads `${{ item }}`.
  - *Status:* ☐ unverified
- **O2 — `.index` unconditional.** The synthetic `.index` is injected for every element type; `${{ item.index }}` / `${{ <alias>.index }}` yields the position for scalar and array elements, not just objects.
  - *Evidence:* a scalar-element map test observes `.index`; the element value stays readable.
  - *Status:* ☐ unverified
- **O3 — no regression.** Unknown namespaces still error typed; existing map/eval tests stay green; `cargo fmt --all --check` / `clippy -all-targets --all-features -D warnings` / `nextest` (all prior + new) / `scripts/purity.sh` all clean.
  - *Status:* ☐ unverified
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line.
  - *Status:* ☐ unverified

## Conclusion
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
