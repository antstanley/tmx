# Plan: Conformance gap-closure (TMX runtime)

**Status:** Done · **Layout:** kanban · **Date:** 2026-07-07 · **Owner:** Ant Stanley · **Source:** the R2 spec-conformance review of the `tmx-runtime` build against [`.specs/`](../../00-overview.md)

Close the **code** divergences the R2 conformance review found (spec is right, code is missing/incorrect). Spec-only drift (stale module trees, "no Rust code exists yet", etc.) is handled separately by a spec-doc refresh, not here. Items that are spec over-reach (the `tmx flow run` noun-group, raising the state-size cap above its hard ceiling, the relaxed newer-spec-version warning) are routed to the doc pass as spec trims, not implemented here.

Built on the same gated workflow as the 33-task runtime build (opus implementer, fable verifier, one jj commit per task on the `tmx-runtime` bookmark).

## Tasks

| Task | Severity | Closes (review finding) |
|---|---|---|
| 34 · layered config + env into `tmx run` | HIGH | config layering consumed only by `list`; `TMX_CONCURRENCY`/`TMX_MAX_STATE_SIZE`/project-user-system layers/`--profile`/`TMX_NO_ENV`/`TMX_INPUT_<NAME>` inert for `run` |
| 35 · fan-out binding fidelity | MED | `as:` map alias not honoured (binds under `item`); element `.index` injected only for object elements |
| 36 · reference-form context/environment execution | MED | reference-form context/env preflight green but fail-closed (exit-4) at the `EngineRunFlow` re-load |
| 37 · adapter/scorer fidelity | MED | `store` per-task `timeout` not enforced; `llmRubric` `apiUrl`/`apiKey` ignored |

Order 34…37; each is largely independent but builds on the accumulating tip. Every task inherits the repo definition of done (tests incl. negative space, `cargo fmt`/`clippy -D warnings`/`nextest` clean, `scripts/purity.sh` green, named-constant limits) and ends with a `Reviewable:` line exercised through the real `tmx` binary.
