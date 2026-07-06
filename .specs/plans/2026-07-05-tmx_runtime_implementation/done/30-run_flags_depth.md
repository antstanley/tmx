# Task 30 — Full `tmx run` flag surface

**Plan:** [plan.md](../plan.md) · **Certificate:** [30-run_flags_depth-certificate.md](30-run_flags_depth-certificate.md)

**Implements:** [07-cli.md](../../../07-cli.md) §`tmx run` (run flags), §Matrix sugar
**Depends on:** 14, 17, 18
**Produces:** the complete `tmx run` flag surface — inputs, env overrides, state seed/dump, task slicing, dry-run, matrix, concurrency, max-state-size, continue-on-error, and watch
**Pointers:** `crates/tmx-cli/src/args.rs`, `crates/tmx-cli/src/commands/run.rs`, `crates/tmx-core/src/runner.rs` (slicing + matrix lowering)

## Steps

- [x] Implement input supply (`--input k=v` / `k:=<json>` / `--inputs-file`) coerced to each declared `type`, `--env K=V` overrides, and `--state-in`/`--state-out` with round-trip re-validation of seeded state on read.
- [x] Implement task slicing (`--only`/`--skip`/`--from`/`--until`) over the sequential list, paired with `--state-in` so later tasks still read prior state, and `--dry-run`/`-n` (resolve + validate + print the plan, execute nothing).
- [x] Implement `--matrix key=v1,v2` lowering to a bounded `map` (repeatable axes cross-product, each binding `${{ matrix.<key> }}`), where an authored `map` wins and `--matrix` is ignored with a stderr warning.
- [x] Wire `--concurrency`, `--continue-on-error`, `--max-state-size`, and `--watch` (each re-run a full run with its own record; SIGINT stops the watcher; exit is the most recent run's code).

## Definition of done

- [x] Each flag affects the run as specified: typed inputs coerce, slicing pairs with `--state-in`, `--dry-run` executes nothing, and `--matrix` produces the cross-product.
- [x] An authored `map` is never rewritten by `--matrix` (a stderr warning is emitted instead), and a `--state-in` file failing re-validation is rejected (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run one flow with `--dry-run`, with `--matrix` on two axes, and with `--from`/`--state-in`, confirming the plan, the cross-product, and the resumed slice.
