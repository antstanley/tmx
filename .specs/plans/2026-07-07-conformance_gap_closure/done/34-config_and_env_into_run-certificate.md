# Done Certificate — Task 34: Layered config + env into `tmx run`

**Task:** [34-config_and_env_into_run.md](34-config_and_env_into_run.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-07

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — layered config binds `run`.** `tmx run` resolves `concurrency` and `max_state_size` as `flag > TMX_CONCURRENCY|TMX_MAX_STATE_SIZE > project > user > system > default`, with `--profile` selecting a layer.
  - *Evidence:* a test and a real `tmx run` where a `TMX_CONCURRENCY` / config-layer value changes the effective cap, and an explicit flag overrides it; `--profile` observed to select a layer.
  - *Status:* ☑ SATISFIED — `resolve_overrides` (run.rs:82) folds `--profile`/`--concurrency`/`--max-state-size` into the highest config layer, then `load_effective` folds `TMX_*` env > project(+profile) > user > system, and `resolve_concurrency`/`resolve_max_state_size` read the result; `run_once` binds them via `check_concurrency(overrides.concurrency)` and `RunConfig.{concurrency_cap,max_state_size_bytes}`. Integration test `concurrency_precedence_flag_beats_env_beats_profile_beats_project` PASS (bare run with malformed project-layer `concurrency` → exit 2 proves the layer binds the run; `--concurrency 4`, `TMX_CONCURRENCY=4`, and `--profile ok` each override to exit 0), and unit `resolve_concurrency_honours_the_layer_precedence` PASS. Env-over-project precedence confirmed directly on the real binary.
- **O2 — env parity.** `TMX_NO_ENV` acts as `--no-env` (explicit flag wins); `TMX_INPUT_<NAME>` supplies a declared input coerced to type, ranked below `--input`/`--inputs-file`.
  - *Evidence:* `TMX_INPUT_FOO=bar` → `${{ inputs.foo }}` == `bar`; `--input foo=baz` overrides to `baz`; `TMX_NO_ENV` suppresses env exposure.
  - *Status:* ☑ SATISFIED — `coerce_inputs` (run.rs:488) scans `TMX_INPUT_<NAME>` for declared inputs only, inserting at lowest precedence so `--inputs-file`/`--input` (inserted after) override; `resolve_local` (config.rs:509) ORs `--local`/`--no-env` with `TMX_NO_ENV`, gating the provider lifecycle at run.rs:227. Integration tests `tmx_input_env_reaches_state_and_an_explicit_input_overrides_it` and `tmx_no_env_suppresses_the_provider_lifecycle_as_local_does` PASS; independently reproduced on the real binary: `TMX_INPUT_FOO=bar`→`bar`, `+ --input foo=baz`→`baz`.
- **O3 — negative space + no regression.** A malformed numeric env value (`TMX_CONCURRENCY=x`) is a typed usage error (exit 2), not silently ignored; the whole prior suite stays green; the already-wired `TMX_FORMAT`/`TMX_NO_COLOR`/`TMX_FLOW`/`TMX_RUNS_RETENTION` resolvers still work.
  - *Evidence:* `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo nextest run` (all prior + new), `scripts/purity.sh` — all clean; a bad env value observed to exit 2.
  - *Status:* ☑ SATISFIED — verifier ran: `cargo fmt --all --check` exit 0; `cargo clippy --all-targets --all-features -- -D warnings` exit 0 (no warnings); `cargo nextest run` = 460 tests, 460 passed, 0 skipped (one unit test flagged `leaky` by nextest's handle-leak heuristic but PASS — harmless, not a failure); `scripts/purity.sh` green. Malformed `TMX_CONCURRENCY=x` and `TMX_MAX_STATE_SIZE=big` each observed to exit 2 with empty stdout on the real binary (integration `a_malformed_numeric_env_value_is_a_usage_error_exit_two` + `resolve_*_rejects_garbage` unit tests PASS); typed `ConfigUsageError` mapped to `EXIT_USAGE=2` at main.rs before the run starts. Named limits reused (`CONCURRENCY_MAX`/`STATE_SIZE_MAX_BYTES`); no new engine bound.
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line.
  - *Status:* ☑ SATISFIED — verifier drove `target/debug/tmx run` directly: `TMX_INPUT_FOO=bar` → `${{ inputs.foo }}` == `bar`; `--input foo=baz` → `baz`; malformed `TMX_CONCURRENCY`/`TMX_MAX_STATE_SIZE` → exit 2; project-config `concurrency` binds the run (integration test). All observed as documented.

## Conclusion
VERDICT: DONE
CONFIDENCE: high
SUMMARY: The documented layered config (flag > `TMX_*` env > project(+`--profile`) > user > system) binds `tmx run`: `resolve_overrides` folds flags as the top layer, `load_effective` folds the rest, and `resolve_concurrency`/`resolve_max_state_size`/`resolve_local` bind the caps and the `local` gate; `coerce_inputs` reads `TMX_INPUT_<NAME>` at lowest precedence; malformed numerics are a typed usage error → exit 2 up front. All four obligations SATISFIED — fmt/clippy/nextest(460)/purity clean, and every reviewable scenario reproduced on the real binary. (Note: the implementer self-report claimed edits to `env_layer`/`apply_profile`/`active_profile`, but those already carried the `TMX_CONCURRENCY`/`TMX_MAX_STATE_SIZE`/`TMX_PROFILE` mappings at HEAD; a self-report inaccuracy only, the delivered state is correct.)
