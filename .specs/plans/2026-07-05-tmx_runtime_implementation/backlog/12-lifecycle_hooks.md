# Task 12 — Lifecycle hooks (one level deep)

**Plan:** [plan.md](../plan.md) · **Certificate:** [12-lifecycle_hooks-certificate.md](12-lifecycle_hooks-certificate.md)

**Implements:** [04-execution-engine.md](../../../04-execution-engine.md) §Lifecycle hooks; [01-domain-model.md](../../../01-domain-model.md) §Pipeline lifecycle (hooks)
**Depends on:** 11
**Produces:** the `HookRunner` firing `create`/`change`/`destroy`/`error` through the same runner, one level deep, with the no-hook-inside-a-hook guarantee asserted
**Pointers:** `crates/tmx-core/src/hooks.rs` (new), `crates/tmx-core/src/runner.rs`

## Steps

- [ ] Run each hook body through the same `PipelineRunner` so a hook inherits the full task model, bounding hook task count by `HOOK_TASKS_MAX`.
- [ ] Fire `create` once on entry to `running`, `change` once per state-changing task (and only when the merge changed the state — a skipped task does not fire it), `error` when a task aborts the Pipeline, and `destroy` on every terminal status like a `finally`.
- [ ] Enforce one-level depth: the runner refuses to fire a lifecycle hook when already inside one, asserted, so a `change` hook that mutates state does not re-trigger `change`.
- [ ] Emit `hook.start`/`hook.finish` events and integrate hook firing into the runner's loop and finish path.

## Definition of done

- [ ] `create`/`change`/`destroy`/`error` fire at exactly the specified transitions over the fakes, `change` fires once per state-changing task and not on a skip, and `destroy` fires on success, failure, and cancellation.
- [ ] A hook whose body would fire another lifecycle hook trips the one-level assertion (negative space: no hook-storm), and an over-`HOOK_TASKS_MAX` hook body is rejected.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run a flow with all four hooks over the fakes and confirm the `hook.start`/`hook.finish` sequence and the single `change` per state-changing task.
