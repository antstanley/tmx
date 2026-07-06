# Task 34 — Layered config + env into `tmx run`

**Plan:** [plan.md](../plan.md) · **Certificate:** [34-config_and_env_into_run-certificate.md](34-config_and_env_into_run-certificate.md)

**Implements:** [07-cli.md](../../../07-cli.md) §Configuration, §Run flags
**Depends on:** — (builds on the completed runtime)
**Produces:** the documented layered config (flag > `TMX_*` env > project > user > system, with `--profile`) actually binds `tmx run` — `--concurrency`/`--max-state-size` fall back through `TMX_CONCURRENCY`/`TMX_MAX_STATE_SIZE` and the config layers/profile; `TMX_NO_ENV` and `TMX_INPUT_<NAME>` are honoured — all exercised end-to-end.
**Pointers:** `crates/tmx-cli/src/config.rs` (`load_effective`/`EffectiveConfig`/`env_layer` — today consumed only by `list.rs:127`), `crates/tmx-cli/src/commands/run.rs:140,151,154` (reads `args.concurrency`/`args.max_state_size` straight from flags), `:440-492` (`coerce_inputs`), `crates/tmx-cli/src/compose.rs:111-117` (`Composed::new`), `crates/tmx-cli/src/main.rs:52` (`--profile` never threaded to run).

## Steps

- [x] Thread `config::load_effective` (with the active `--profile`) into the `run` path: resolve `concurrency` and `max_state_size` as `flag ?? TMX_CONCURRENCY|TMX_MAX_STATE_SIZE ?? project/user/system layer ?? built-in default`, and pass the resolved values into `RunConfig`/`Composed` — matching the precedence 07-cli.md §Configuration documents. Keep the existing already-wired resolvers (`TMX_FORMAT`/`TMX_NO_COLOR`/`TMX_FLOW`/`TMX_RUNS_RETENTION`) working.
- [x] Honour `TMX_NO_ENV` as the env-var equivalent of `--no-env`/`--local` (fold into the effective `local` default; explicit flag still wins).
- [x] Honour `TMX_INPUT_<NAME>` in `coerce_inputs` — scan the environment for declared inputs, coerced to the declared type, ranked **below** an explicit `--input`/`--inputs-file` value.
- [x] Add tests: the env vars and config layers change a run's behaviour (a `TMX_CONCURRENCY` cap observed, a `TMX_INPUT_x` value reaching state, `--input` overriding it, `--profile` selecting a layer) and precedence holds; a bad `TMX_MAX_STATE_SIZE`/`TMX_CONCURRENCY` value is rejected as a usage error (exit 2), not silently ignored.

## Definition of done

- [x] `tmx run` honours `TMX_CONCURRENCY`, `TMX_MAX_STATE_SIZE`, the project/user/system config layers, and `--profile` for concurrency and max-state-size, with `flag > env > project > user > system` precedence.
- [x] `TMX_NO_ENV` and `TMX_INPUT_<NAME>` are read on the run path (explicit flags/inputs win); a malformed numeric env value is a typed usage error, not silently dropped.
- [x] Meets the repo definition of done (tests incl. negative space, `cargo fmt`/`clippy -D warnings`/`nextest`/`scripts/purity.sh` clean; no new hard-coded bound — reuse named limits).
- [x] Reviewable: from the shell, set `TMX_CONCURRENCY=2` (and a project config) and observe it bound a map fan-out on a `tmx run`; set `TMX_INPUT_FOO=bar` and observe `${{ inputs.foo }}` = `bar`, then override with `--input foo=baz` and observe `baz`.
