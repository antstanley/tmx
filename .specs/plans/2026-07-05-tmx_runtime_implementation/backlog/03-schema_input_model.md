# Task 03 — Schema input model (the static Flow)

**Plan:** [plan.md](../plan.md) · **Certificate:** [03-schema_input_model-certificate.md](03-schema_input_model-certificate.md)

**Implements:** [01-domain-model.md](../../../01-domain-model.md) §Input entities (the static Flow — `tmx-schema`), §Tasks, §Context/Environment/InputSpec; [02-crate-architecture.md](../../../02-crate-architecture.md) §tmx-schema (`flow.rs`, `task.rs`, `context.rs`, `environment.rs`); the input contract [`tmx.schema.json`](../../../../docs/tmx.schema.json)
**Depends on:** 01, 02
**Produces:** the deserialize-only Rust mirror of every `tmx.schema.json` `$def`, so the whole example corpus loads into typed values with source order preserved
**Pointers:** `crates/tmx-schema/src/flow.rs` (new), `crates/tmx-schema/src/task.rs` (new), `crates/tmx-schema/src/context.rs` (new), `crates/tmx-schema/src/environment.rs` (new), `docs/examples/` (the corpus to round-trip)

## Steps

- [ ] Define `Flow` (optional `name`/`description`/`version`/`environment`/`context`/`inputs`, required `tasks`), with `environment` and `context` each an `Inline(Box<…>) | Reference(String)`, and `InputSpec` (`type`/`description`/`required`/`default`).
- [ ] Define `Tasks` as `List(Vec<Task>) | Map(IndexMap<String, TaskEntry>)` and `TaskEntry` as `Task(Box<Task>) | Shorthand(String)`, using `indexmap` so the map form preserves source key order as a type property.
- [ ] Define the `Task` envelope and the `TaskWith` enum discriminated by `type` over all ten variants, boxing `MapWith`/`EvalWith` (they embed a `Task`); derive `Deserialize` and keep the crate I/O-free.
- [ ] Define `Context` (`env`/`secrets`/`hooks`), `Hook`, `SecretSource`, `Environment` as an open object (`#[serde(flatten)] extra`), and `Duration` as `Seconds(u64) | Spec(String)`.
- [ ] Add a corpus round-trip test that deserializes every example artifact and asserts array-form and map-form task lists both preserve document order.

## Definition of done

- [ ] Every `$def` in [`tmx.schema.json`](../../../../docs/tmx.schema.json) has a corresponding type, and every example in `docs/examples/` deserializes without loss.
- [ ] A map-form Flow and an array-form Flow both round-trip with tasks in source order, and a malformed `with`/`type` pairing fails to deserialize (negative space).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the corpus round-trip test and diff a re-serialized map-form Flow to confirm key order is preserved.
