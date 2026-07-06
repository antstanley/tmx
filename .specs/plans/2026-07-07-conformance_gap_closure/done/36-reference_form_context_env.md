# Task 36 — Reference-form context/environment execution

**Plan:** [plan.md](../plan.md) · **Certificate:** [36-reference_form_context_env-certificate.md](36-reference_form_context_env-certificate.md)

**Implements:** [03-loading-and-preflight.md](../../../03-loading-and-preflight.md) §Reference resolution, §Preflight; [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Context/Environment
**Depends on:** —
**Produces:** a Flow whose `context` or `environment` is a **reference form** (an external-file `use:`/`$ref`) runs end-to-end via `tmx run` — not just green at preflight — collecting the resolved context/environment into the run exactly like an inline form.
**Pointers:** `crates/tmx-core/src/preflight.rs` (`resolve_references` — already inlines reference-form context/env before `resolve()`, so preflight passes), `crates/tmx-core/src/resolve.rs` (the `EngineRunFlow` re-load path where a reference-form context/environment currently fails-closed with a typed exit-4), `crates/tmx-core/src/usecases.rs` (`EngineRunFlow`).

## Steps

- [x] Trace why a reference-form context/environment flow that preflights green then fails at the `EngineRunFlow` re-load in `resolve.rs` (the re-load resolves the flow a second time without the reference inlining preflight did, so the unresolved reference trips a typed error). Fix the re-load so it carries/repeats the same reference resolution — reuse the preflight-resolved `ResolvedFlow` (preferred: do not re-load/re-resolve from scratch), or apply the identical `resolve_references` inlining on the re-load path.
- [x] Ensure no double-resolution regressions: an inline-form flow still runs identically, and the reference resolution stays bounded (kind dispatch, JSON-depth bound, schema validation, `cyclic_flow_import` guard) exactly as preflight applies it.
- [x] Add tests: a reference-form `context` flow and a reference-form `environment` flow each run end-to-end over the fakes and via the real binary, producing the same final state as their inline equivalents; a dangling reference still fails typed.

## Definition of done

- [x] A Flow with a reference-form (external-file) `context` runs end-to-end (exit 0) with the referenced context available to tasks; likewise a reference-form `environment`. Neither fails-closed at the engine re-load.
- [x] Inline-form context/environment flows are unchanged; a dangling/cyclic reference still surfaces its typed error.
- [x] Meets the repo definition of done (tests incl. negative space, `cargo fmt`/`clippy -D warnings`/`nextest`/`scripts/purity.sh` clean).
- [x] Reviewable: `tmx run` a flow whose `context` is `{ use: ./ctx.yaml }` (external file) and observe a task read a value from that context in the final state, exit 0 — not exit 4.
