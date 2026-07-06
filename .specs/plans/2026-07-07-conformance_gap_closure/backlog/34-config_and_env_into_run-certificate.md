# Done Certificate — Task 34: Layered config + env into `tmx run`

**Task:** [34-config_and_env_into_run.md](34-config_and_env_into_run.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-07 — unverified

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — layered config binds `run`.** `tmx run` resolves `concurrency` and `max_state_size` as `flag > TMX_CONCURRENCY|TMX_MAX_STATE_SIZE > project > user > system > default`, with `--profile` selecting a layer.
  - *Evidence:* a test and a real `tmx run` where a `TMX_CONCURRENCY` / config-layer value changes the effective cap, and an explicit flag overrides it; `--profile` observed to select a layer.
  - *Status:* ☐ unverified
- **O2 — env parity.** `TMX_NO_ENV` acts as `--no-env` (explicit flag wins); `TMX_INPUT_<NAME>` supplies a declared input coerced to type, ranked below `--input`/`--inputs-file`.
  - *Evidence:* `TMX_INPUT_FOO=bar` → `${{ inputs.foo }}` == `bar`; `--input foo=baz` overrides to `baz`; `TMX_NO_ENV` suppresses env exposure.
  - *Status:* ☐ unverified
- **O3 — negative space + no regression.** A malformed numeric env value (`TMX_CONCURRENCY=x`) is a typed usage error (exit 2), not silently ignored; the whole prior suite stays green; the already-wired `TMX_FORMAT`/`TMX_NO_COLOR`/`TMX_FLOW`/`TMX_RUNS_RETENTION` resolvers still work.
  - *Evidence:* `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo nextest run` (all prior + new), `scripts/purity.sh` — all clean; a bad env value observed to exit 2.
  - *Status:* ☐ unverified
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line.
  - *Status:* ☐ unverified

## Conclusion
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
