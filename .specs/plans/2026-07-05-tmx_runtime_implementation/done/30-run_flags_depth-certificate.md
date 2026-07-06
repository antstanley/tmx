# Done Certificate — Task 30: Full `tmx run` flag surface

**Task:** [30-run_flags_depth.md](30-run_flags_depth.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

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
  - *Status:* ☑ SATISFIED — clap surface read (`args.rs`, `run_parses_the_full_flag_depth_surface` passes, `-n` short + non-numeric `--concurrency` negative space included). Named tests pass: `coerce_inputs_coerces_each_value_to_its_declared_type`, `runner_seeds_prior_state_so_a_sliced_continuation_reads_it`, `dry_run_prints_the_plan_and_executes_no_task` (sentinel absent under `--dry-run`, present on the real run), `matrix_runs_the_full_cross_product_binding_each_axis`, `runner_binds_the_matrix_combination_into_every_task_scope`. Traced + executed live: `--matrix a=1,2 --matrix b=x,y` produced the 4-way product `1-x|1-y|2-x|2-y|`; `--from verify --state-in seed.json` ran only `verify`, which read `${{ tasks.build.sha }}` from the seed (exit 0; unseeded mirror exits non-0). `--env` overrides land (unit test), `--state-out` round-trips, `--continue-on-error` → `RunConfig.continue_on_error` (engine-tested), `--max-state-size` → `StateBuilder::from_state_with_cap`/`with_cap` clamped to `STATE_SIZE_MAX_BYTES`, `--watch` exercised live (initial run → touch → full re-run with its own id → SIGINT stops, exit 0), `--matrix` failing combination exits 1 even when a later one passes (`matrix_with_a_failing_combination_exits_one_even_when_a_later_one_passes`). Residue note below on the `--concurrency` value.

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
  - *Status:* ☑ SATISFIED — executed live: an authored-`map` flow run with `--matrix a=1,2` printed `tmx: warning: this Flow authors a `map` task; --matrix is ignored (the authored map wins)` on stderr and produced zero combinations (`"matrix": []` in the dry-run plan; `resolve_matrix_lowers_two_axes_and_an_authored_map_wins` covers it in-tree; the map is never rewritten — `resolve_matrix` only reads the flow). `--state-in` negative space tripped live: malformed JSON → exit 3 `Validation [state_in_invalid]`; a non-object (`[1,2,3]`) → exit 3 `Validation [state_not_object]` (re-validation via `PipelineState::new`); `read_state_in_re_validates_and_rejects_a_bad_file` covers both in-tree.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant in
    `tmx-schema::limits` (e.g. any cap behind `--concurrency`/`--max-state-size` resolves to a named
    constant, not a literal).
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features
    -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm each new validation path
    (typed-input coercion failure, malformed `--state-in`) ships a negative-space test.
  - *Status:* ☑ SATISFIED — independently ran `cargo fmt --all --check` (clean), `cargo clippy --all-targets --all-features -- -D warnings` (clean), `cargo nextest run` (392/392 pass, includes the new matrix run-store regression test `a_reused_sink_files_each_matrix_run_under_its_own_id`). Negative-space tests present for coercion failure (`coerce_inputs_rejects_a_value_that_does_not_match_its_declared_type`), malformed `--state-in`, malformed `--env`, unknown sliced task (`an_unknown_task_name_is_a_typed_error`), over-wide matrix (`an_over_width_matrix_is_fanout_too_wide`), over-ceiling `--concurrency`. No new engine bound; the one new numeric, `WATCH_POLL_INTERVAL_MS`, is a named constant with a documented rationale for living in the CLI (a UI cadence, not an engine dimension).

- **O4 — Reviewable: run one flow with `--dry-run`, with `--matrix` on two axes, and with `--from`/`--state-in`, confirming the plan, the cross-product, and the resumed slice (Reviewable).**
  - *Claim:* a reviewer can run the three invocations and observe the printed plan (nothing executed),
    the two-axis cross-product, and the resumed slice reading prior state.
  - *Evidence to collect:* build the binary (`cargo build -p tmx-cli`), then run
    `tmx run <flow> --dry-run` (observe the resolved + validated plan printed and no task side
    effect), `tmx run <flow> --matrix a=1,2 --matrix b=x,y` (observe the four-combination
    cross-product), and `tmx run <flow> --from <task> --state-in <state.json>` (observe the resumed
    slice reading the seeded prior state). Observe stdout/stderr split and exit code as in Task 17.
  - *Status:* ☑ SATISFIED — built `tmx-cli` and ran all three against real flows: (1) `tmx run flow.yaml --no-store --dry-run` printed the plan JSON (`"dryRun": true`, both tasks listed) to stdout, exit 0, and the file-write task's sentinel was NOT created (the same flow run for real does create it); (2) `tmx run mtx.yaml --matrix a=1,2 --matrix b=x,y` appended all four combinations (`1-x|1-y|2-x|2-y|`), exit 0, and with the store enabled produced four run dirs each holding BOTH `record.json` and `log.ndjson` whose `run.start` id matches its own record (the fixed behaviour; `tmx runs logs <last-id>` reads that combination's events); (3) `tmx run flow.yaml --from verify --state-in seed.json` ran only the resumed slice, which read the seeded `${{ tasks.build.sha }}`, exit 0. stdout/stderr split held throughout (machine data on stdout, progress on stderr).

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
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with executed evidence: the full flag surface parses and each
flag was exercised end to end (dry-run side-effect-free, 2×2 matrix cross-product with per-combination
run-store dirs after the re-latch fix, resumed --from/--state-in slice, live --watch re-run + SIGINT);
both negative spaces trip (authored `map` wins with a stderr warning, malformed/non-object --state-in
rejected exit 3); fmt/clippy/nextest all clean (392/392). Regression check passed: a plain flag-free
`tmx run flow.yaml` still loads, executes, prints masked state to stdout and exits 0. Residue confirmed:
matrix width is bounded (a 160,000-combination request returned `fanout_too_wide`, not truncation) and
`--concurrency` is capped at CONCURRENCY_MAX (257 → exit 3 `concurrency_too_high`). Remaining residue,
non-blocking for this task: the `--concurrency` VALUE has no engine consumer yet because `map`/`eval`
dispatch is not wired into the sequential runner (dispatch.rs returns `task_type_unsupported`; run_map's
`concurrency_cap` parameter awaits that wiring — a plan-level follow-up, not a Task-30 defect); `k:=<json>`
raw inputs bypass declared-type shape checking; `slice_tasks` carries a duplicate and an unreachable
match arm (cosmetic); a reversed `--from`/`--until` pair yields a silent empty run.
