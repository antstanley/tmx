# Task 15 — Preflight (load → resolve → validate → capability)

**Plan:** [plan.md](../plan.md) · **Certificate:** [15-preflight-certificate.md](15-preflight-certificate.md)

**Implements:** [03-loading-and-preflight.md](../../../03-loading-and-preflight.md) §Responsibilities, §Directory assembly, §Validation, §Capability check, §Preflight flow; [04-execution-engine.md](../../../04-execution-engine.md) §Bounded `flow` recursion (cycle backstop)
**Depends on:** 04, 13, 14
**Produces:** the `tmx-core` preflight orchestration that either passes wholesale to a `ResolvedFlow` + `CapabilitySet` or fails fast with nothing executed
**Pointers:** `crates/tmx-core/src/preflight.rs` (new), `crates/tmx-schema/src/` (desugar helpers), `crates/tmx-core/src/usecases.rs` (`ValidateArtifacts`/`InspectFlow`)

## Steps

- [ ] Orchestrate load → `kind` dispatch → reference resolution → directory assembly → desugar → validate → capability check via the `SourceLoader`/`ReferenceResolver`/`SchemaValidator` ports.
- [ ] Assemble a directory into one Flow: a sibling `environment.*`/`context.*` becomes the shared context (same-folder only), every other artifact a task, ordered by natural filename order (byte-wise ASCII, case-sensitive, with maximal digit runs compared as unsigned integers); desugar the map form and `exec` shorthand into an ordered `Vec<Task>`, and normalise every `Duration` (`Seconds`/`Spec` such as `"30s"`) to milliseconds at this resolution step so adapters receive normalised timeouts.
- [ ] Enforce the preflightable limits as `Validation` errors (`too_many_tasks`, literal `fanout_too_wide`, `json_too_deep`, `concurrency_too_high`, `too_many_hook_tasks`) and the structural checks (`missing_task_name`, duplicate-name `ResolutionError`); emit the newer-spec compatibility warning and reject an unknown construct as `Validation`.
- [ ] Compute the `CapabilitySet` (recursing into `map`/`eval` inner tasks, `eval` scorer kinds, hook bodies, provider methods) and return `EnvironmentError` (`missing_capability`) when a required adapter is absent or a denying stub.

## Definition of done

- [ ] A passing preflight yields a `ResolvedFlow` (ordered `Vec<Task>`, resolved env/context) and a `CapabilitySet`; a directory with one malformed task aborts before any task executes.
- [ ] Over-limit counts/widths/depths and a nameless or duplicate-named task are rejected at preflight, and a Flow needing an unwired capability returns `missing_capability` naming the port and task type (negative space).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: preflight a directory of task files and confirm natural-filename ordering, and run the fail-fast tests for a malformed task and a missing capability.
