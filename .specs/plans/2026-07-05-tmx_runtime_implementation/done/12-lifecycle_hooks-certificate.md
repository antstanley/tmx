# Done Certificate — Task 12: Lifecycle hooks (one level deep)

**Task:** [12-lifecycle_hooks.md](12-lifecycle_hooks.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — discharged by an independent verifier over the fakes

> This certificate is a verification protocol for Task 12. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 12) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion. O4 is the
Reviewable item; record DONE only when O1…O4 are all SATISFIED.

## Premises

- **P1 — Goal.** Produce the `HookRunner` firing `create`/`change`/`destroy`/`error` through the same
  runner, one level deep, with the no-hook-inside-a-hook guarantee asserted.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This task extends the Task 11 `PipelineRunner::run` loop and finish path. The
  Task 11 runner integration test must keep passing: a hook-free flow runs exactly as before, and the
  four composed pure services (07/08/09/10) are unaffected. Hooks add firing points; they do not
  change the base loop's task semantics.

## Obligations

- **O1 — `create`/`change`/`destroy`/`error` fire at exactly the specified transitions over the fakes, `change` fires once per state-changing task and not on a skip, and `destroy` fires on success, failure, and cancellation.**
  - *Claim:* `create` fires once on entry to `running`; `change` fires once per state-changing task
    and only when the merge actually changed the state (a skipped `if=false` task does not fire it);
    `error` fires when a task aborts the Pipeline; and `destroy` fires on every terminal status —
    success, failure, and cancellation — like a `finally`.
  - *Evidence to collect:* read `crates/tmx-core/src/hooks.rs` and the hook integration points in
    `crates/tmx-core/src/runner.rs`; run `cargo nextest run -p tmx-core hooks` and confirm tests that
    each hook fires at its transition over the fakes, that `change` fires once per state-changing task
    and not on a skip, and that `destroy` fires on the success, failure, and cancellation terminal
    paths.
  - *Checks:* trace a skipped task (`if=false`, no merge) and confirm the `change` fire-path is not
    reached; trace a state-changing task and confirm `change` fires exactly once after its merge, not
    per-iteration.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core -E 'binary(hooks)'`: 9/9 pass.
    `all_four_hooks_fire_at_their_transitions_in_order` shows create → change → error → destroy;
    `change_fires_once_per_state_changing_task_and_never_on_a_skip` (2 fires, none for the gated skip)
    and `change_does_not_fire_when_the_merge_did_not_change_state` cover the skip/no-op clauses;
    `destroy_fires_on_both_success_and_failure` covers success and failure. Cancellation: the runner
    cannot reach `cancelled` until Task 29; verified by code reading that the `destroy` fire in
    `PipelineRunner::run` is on the unconditional post-loop finally path (no status branch to stub) and
    `destroy_fires_through_the_status_independent_finally_path` drives that exact path — representative,
    not stubbed. Skip trace confirmed: `StepOutcome::Skipped` arm never reaches the merge/`change` code.

- **O2 — A hook whose body would fire another lifecycle hook trips the one-level assertion, and an over-`HOOK_TASKS_MAX` hook body is rejected.**
  - *Claim:* when already inside a hook, the runner refuses to fire a lifecycle hook — asserted — so a
    `change` hook that mutates state does not re-trigger `change` (no hook-storm, negative space); and
    a hook body with more than `HOOK_TASKS_MAX` tasks is rejected.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core hooks` and confirm the nested-hook test
    (a hook body that would fire another lifecycle hook) trips the one-level assertion, and the
    over-`HOOK_TASKS_MAX` hook-body test is rejected with the typed error.
  - *Checks:* trace a `change` hook body that mutates state and confirm the in-hook guard asserts (or
    the "inside a hook" flag suppresses firing) before a second lifecycle hook can fire, so recursion
    into a hook-within-a-hook is impossible.
  - *Status:* ☑ SATISFIED — `firing_a_hook_while_already_inside_one_trips_the_assertion` panics with
    "one level deep" (`#[should_panic]`, ran and passed); `a_change_hook_that_mutates_state_does_not_re_trigger_change`
    shows exactly one `change` fire (no storm — the hook body runs on a `for_hook_body` runner whose
    `run_tasks` receives `hooks: None`, so the `change` fire-path is structurally unreachable inside a
    body); `an_over_limit_hook_body_is_rejected` returns the typed `too_many_hook_tasks` error with no
    hook event emitted, bounded by the named `tmx_schema::limits::HOOK_TASKS_MAX`.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the hook-body bound is the
    named `tmx-schema::limits` constant `HOOK_TASKS_MAX`, not a magic number; run the `cargo tree`
    purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no match) confirming `tmx-core`
    stays free of an async-runtime/I/O edge.
  - *Status:* ☑ SATISFIED — independently ran from the main tree: `cargo fmt --all --check` (exit 0),
    `cargo clippy --all-targets --all-features -- -D warnings` (clean), `cargo nextest run`
    (117/117 pass), `scripts/purity.sh` (exit 0: no I/O or async dependency edge). The hook-body bound
    is the named `HOOK_TASKS_MAX` (`tmx-schema/src/limits.rs:138`, compile-time asserted ≥ 1); no new
    numeric literal was introduced.

- **O4 — Reviewable: run a flow with all four hooks over the fakes and confirm the `hook.start`/`hook.finish` sequence and the single `change` per state-changing task (Reviewable).**
  - *Claim:* a reviewer can run a flow exercising all four hooks over the fakes and observe the
    `hook.start`/`hook.finish` event sequence and a single `change` per state-changing task.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core hooks` and read the summary; confirm the
    all-four-hooks test asserts the ordered `hook.start`/`hook.finish` sequence and exactly one
    `change` per state-changing task, with zero failures.
  - *Status:* ☑ SATISFIED — beyond the passing test, the verifier ran the all-four-hooks flow through
    `EngineRunFlow` in an out-of-tree probe and printed the full event stream; observed interleaving:
    `run.start` → hook create (its body's task events nested inside `hook.start`/`hook.finish`) →
    `task.finish one` → hook change → `task.error two` → hook error → hook destroy → `run.finish
    [Failed]`. Hook events nest attributably; exactly one `change` for the one state-changing task.
    A second probe (failing `destroy` body) showed `hook.finish destroy [Error]` with `run.finish [Ok]`
    — a failed hook body does not mask the terminal status.

## Regression check

- This task extends the Task 11 runner run-path (`PipelineRunner::run` loop and finish). Re-run the
  Task 11 integration suite (`cargo nextest run -p tmx-core runner`) and confirm a hook-free flow
  still emits the same event stream and returns the same masked final state as before hooks existed —
  the hook firing points must be no-ops when no hook is declared.

## Residue

- The `destroy`-on-cancellation case needs a cancellation signal; full cancellation is Task 29, so
  confirm the fake path used here to reach the cancelled terminal status is representative, not a
  stubbed-out branch.
- Confirm `error` then `destroy` ordering on an aborting task (error hook fires, then destroy as the
  `finally`), and that a failed hook body itself does not mask the original terminal status.
- Confirm `hook.start`/`hook.finish` nest correctly relative to the surrounding `run`/`task` events,
  so a reader can attribute each hook event to its triggering transition.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently produced evidence. O1–O2/O4 discharged by
running the 9 hook integration tests (9/9 pass) plus two out-of-tree event-stream probes; O3 by the
four gate commands and the purity script, all clean. Regression: `cargo nextest run -p tmx-core
runner` 8/8 pass and `a_hook_free_flow_emits_no_hook_events` shows the firing points are no-ops for
a hook-free flow. Residue items confirmed: error precedes destroy on an abort; a failed hook body's
status lands in `hook.finish` without masking the run's terminal status; hook events nest
attributably between their triggering run/task events; the destroy fire path is genuinely
status-independent (representative of the not-yet-reachable cancelled status, Task 29). Noted, non-
blocking: (a) a `continueOnError` error-record merge changes state but fires no `change` — an
unspecified corner (SCHEMA.md's resolution addresses skips only) worth an explicit spec/test when
Task-29-era semantics land; (b) `Hook::Reference`/`Hook::Use` bodies are a typed
`unsupported_reference` until the loading unit (Tasks 13/15) resolves them, and the HookRunner
struct doc says fire "is a no-op" when `in_hook` while the code asserts — the assert matches the
spec ("refuses … asserted"); the doc phrase could be tightened.
