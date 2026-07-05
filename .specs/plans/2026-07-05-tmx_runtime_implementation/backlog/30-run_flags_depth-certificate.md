# Done Certificate — Task 30: Full `tmx run` flag surface

**Task:** [30-run_flags_depth.md](30-run_flags_depth.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 30. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 30) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship the complete `tmx run` flag surface — inputs, env overrides, state seed/dump,
  task slicing, dry-run, matrix, concurrency, max-state-size, continue-on-error, and watch — each
  behaving per [07-cli.md](../../../07-cli.md) §`tmx run` and §Matrix sugar.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Builds on Task 17 (`tmx run` end-to-end path and its composition root),
  Task 18 (`map` lowering + `Scheduler`), and Task 14 (schema validator, reused to re-validate
  `--state-in`). None of these may regress: the new flags are opt-in additions to the Task-17 path.

## Obligations

- **O1 — Each flag affects the run as specified: typed inputs coerce, slicing pairs with `--state-in`, `--dry-run` executes nothing, `--matrix` produces the cross-product.**
  - *Claim:* `--input k=v` / `k:=<json>` / `--inputs-file` coerce to each declared `type`; `--env K=V`
    overrides context env; `--state-in`/`--state-out` seed/dump `PipelineState`; `--only`/`--skip`/
    `--from`/`--until` slice the sequential task list and pair with `--state-in` so a later task still
    reads prior state; `--dry-run`/`-n` resolves + validates + prints the plan and executes nothing;
    `--matrix key=v1,v2` on repeatable axes lowers to a bounded `map` cross-product binding
    `${{ matrix.<key> }}`; `--concurrency`, `--continue-on-error`, `--max-state-size`, and `--watch`
    thread through as specified.
  - *Evidence to collect:* read `crates/tmx-cli/src/args.rs` for the clap flag definitions; read
    `crates/tmx-cli/src/commands/run.rs` for input coercion, the `--dry-run` short-circuit, and the
    env/state wiring; read `crates/tmx-core/src/runner.rs` for slicing and matrix lowering. Run the
    named tests exercising typed-input coercion, a `--from` slice seeded with `--state-in`, a
    `--dry-run` that performs no task side effect, and a two-axis `--matrix` cross-product — expect
    each to pass.
  - *Checks:* trace `--matrix a=1,2 --matrix b=x,y` lowering to a bounded `map` and confirm it yields
    the 4-way cross-product, each combination binding `${{ matrix.a }}`/`${{ matrix.b }}`; trace a
    `--from`/`--state-in` slice and confirm the resumed task reads a prior task's state via
    `${{ tasks.NAME.field }}`.
  - *Status:* ☐ unverified

- **O2 — An authored `map` is never rewritten by `--matrix` (a stderr warning is emitted instead), and a `--state-in` file failing re-validation is rejected (negative space).**
  - *Claim:* when the target Flow already contains a `map`, `--matrix` is ignored and `tmx run` warns
    on stderr — the CLI never rewrites or wraps an explicit `map`; and a `--state-in` file that fails
    schema re-validation on read is rejected as a typed error, not silently seeded.
  - *Evidence to collect:* read `crates/tmx-core/src/runner.rs` matrix lowering for the authored-`map`
    guard; read `crates/tmx-cli/src/commands/run.rs` for the `--state-in` read path invoking the
    Task-14 `SchemaValidator`. Run the negative tests — an authored-`map` Flow run with `--matrix`
    (expect the `map` unchanged and a stderr warning, `--matrix` ignored) and a malformed `--state-in`
    file (expect rejection with a validation/resolution error, not execution) — expect both to hold.
  - *Checks:* confirm the authored `map` short-circuits `--matrix` with a stderr warning and no
    rewrite; confirm `--state-in` re-validates seeded state on read and rejects a failing file rather
    than trusting state TMX itself may have written.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant in
    `tmx-schema::limits` (e.g. any cap behind `--concurrency`/`--max-state-size` resolves to a named
    constant, not a literal).
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features
    -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm each new validation path
    (typed-input coercion failure, malformed `--state-in`) ships a negative-space test.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run one flow with `--dry-run`, with `--matrix` on two axes, and with `--from`/`--state-in`, confirming the plan, the cross-product, and the resumed slice (Reviewable).**
  - *Claim:* a reviewer can run the three invocations and observe the printed plan (nothing executed),
    the two-axis cross-product, and the resumed slice reading prior state.
  - *Evidence to collect:* build the binary (`cargo build -p tmx-cli`), then run
    `tmx run <flow> --dry-run` (observe the resolved + validated plan printed and no task side
    effect), `tmx run <flow> --matrix a=1,2 --matrix b=x,y` (observe the four-combination
    cross-product), and `tmx run <flow> --from <task> --state-in <state.json>` (observe the resumed
    slice reading the seeded prior state). Observe stdout/stderr split and exit code as in Task 17.
  - *Status:* ☐ unverified

## Regression check

- Task 17 + Task 18: trace that a plain `tmx run flow.yaml` with no flags still loads, preflights,
  executes the `exec`/`assert` tasks, and prints masked final state unchanged — the new flags are all
  opt-in, so their absence must leave the Task-17 path and the Task-18 `map` lowering behaviour intact.

## Residue

- `--watch` semantics (each re-run is a full run with its own record; SIGINT stops the watcher; exit
  is the most recent run's code) and `--concurrency`/`--continue-on-error`/`--max-state-size` are in
  the Produces line but only lightly touched by the O4 reviewable — validator should confirm O1's
  "each flag affects the run as specified" test coverage reaches them, including `--concurrency`
  capping at `CONCURRENCY_MAX` and `--max-state-size` adjusting `STATE_SIZE_MAX_BYTES`.
- `--matrix` cross-product must still respect `FANOUT_WIDTH_MAX` after lowering; check an over-width
  matrix returns `fanout_too_wide` (inherited from Task 18), not a silent truncation.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
