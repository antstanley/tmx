# Done Certificate — Task 03: Schema input model (the static Flow)

**Task:** [03-schema_input_model.md](03-schema_input_model.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

> This certificate is a verification protocol for Task 03. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 03) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The deserialize-only Rust mirror of every `tmx.schema.json` `$def`, so the whole example corpus loads into typed values with source order preserved.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** None — greenfield foundation; no prior behavior to preserve.

## Obligations

- **O1 — Every `$def` in `tmx.schema.json` has a corresponding type, and every example in `docs/examples/` deserializes without loss.**
  - *Claim:* each `$def` in `docs/tmx.schema.json` maps to a Rust type across `flow.rs`/`task.rs`/`context.rs`/`environment.rs`, and every artifact under `docs/examples/` deserializes without loss.
  - *Evidence to collect:* read the planned `crates/tmx-schema/src/{flow,task,context,environment}.rs`; enumerate the `$defs` in [`docs/tmx.schema.json`](../../../../docs/tmx.schema.json) and confirm each has a corresponding type (`Flow`, `InputSpec`, `Tasks`, `TaskEntry`, `Task`, `TaskWith` and its ten variants, `Context`, `Hook`, `SecretSource`, `Environment`, `Duration`). Run the corpus round-trip test in `crates/tmx-schema` that deserializes every file under `docs/examples/` (`single-file-flow.{yaml,json,jsonc,toml}`, `map-tasks.yaml`, `shorthand-tasks.json`, `eval.*`, `map-fanout.*`, `typed-output.*`, `minimal-flow.json`, `folder-layout/`, `standalone/`) and expect all to load.
  - *Status:* ☑ SATISFIED — all 28 `$defs` map 1:1 to a Rust type (`taskList`→`Tasks::List`, `reference`→`String`, `envMap`→`EnvMap`, `matcherName`→existing `MatcherName`, the other 25 to named types across `flow.rs`/`task.rs`/`context.rs`/`environment.rs`). Schema-vs-model audit: every `additionalProperties:false` object carries `deny_unknown_fields`, every open object (`chatCompletionWith`/`environment`/`secretSource`) carries `#[serde(flatten)] extra`, and every schema-required field is non-`Option` — no drift. `every_corpus_artifact_deserialises` PASSED (24 files; 16 flows / 2 contexts / 2 environments / 3 tasks; README+provider skipped). All ten task types incl. kebab-case `chat-completion` appear in the deserialized corpus, exercising every `TaskWith` variant.

- **O2 — A map-form and an array-form Flow both round-trip in source order, and a malformed `with`/`type` pairing fails to deserialize.**
  - *Claim:* array-form and map-form task lists preserve document order through a round-trip, and a `with`/`type` mismatch is rejected at deserialize.
  - *Evidence to collect:* run the corpus round-trip test's order-preservation assertions on an array-form Flow and a map-form Flow (e.g. `map-tasks.yaml`) and confirm tasks emerge in source order. Add or confirm a negative test that a `Task` whose `with` payload does not match its `type` discriminant fails to deserialize.
  - *Checks:* confirm the map form (`Tasks::Map` / `TaskEntry`) deserializes into `indexmap::IndexMap`, not `std::collections::HashMap` or `serde_json::Map` — either of which would silently lose source key order.
  - *Status:* ☑ SATISFIED — `array_form_flow_preserves_source_order` (six tasks in document order) and `map_form_flow_preserves_source_order` (map-tasks.yaml keys in source order) PASS. The load-bearing witness `map_form_preserves_unsorted_order_pinning_indexmap` uses `zebra,alpha,mango` — an order a `BTreeMap`/`HashMap` cannot reproduce — pinning the map form to `indexmap::IndexMap` (confirmed by reading `flow.rs`: `Tasks::Map(IndexMap<String, TaskEntry>)`). Negative space `a_mismatched_with_and_type_fails_to_deserialise` PASSES its 4 cases (exec+fetch payload, fetch+exec payload, unknown type, missing `with`); I independently added 2 fresh mismatches (store+exec payload, store+unknown field) and confirmed both are rejected, then reverted.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-schema` pulls in no async runtime or I/O crate. Task 03 changes no schema or example (the corpus is consumed read-only), so `scripts/validate.sh` is not required by this task.
  - *Status:* ☑ SATISFIED — I independently ran, from the repo root: `cargo fmt --all --check` (exit 0), `cargo clippy --all-targets --all-features -- -D warnings` (exit 0, clean), `cargo nextest run` (13/13 passed across 7 binaries), and `bash scripts/purity.sh` (green: tmx-schema/tmx-core/tmx-testkit carry no I/O or async edge — the YAML/TOML parsers are dev-only). No new runtime bound is introduced (this is a deserialize-only model; `concurrency`/`retries`/`maxTokens` parse as plain integers, validated against the existing `limits` constants at later tasks).

- **O4 — Reviewable: run the corpus round-trip test and diff a re-serialized map-form Flow to confirm key order is preserved.**
  - *Claim:* a reviewer can run the corpus round-trip test and diff a re-serialized map-form Flow against its source to observe key order preserved.
  - *Evidence to collect:* run `cargo nextest run -p tmx-schema`; then re-serialize a map-form example (e.g. `map-tasks.yaml`) and diff the task key order against the source, observing identical order.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-schema` PASSES (9 tests). The crate is `Deserialize`-only, so per the Residue the O4 intent is discharged by a structural key-order assertion rather than a re-emit: `map_form_flow_preserves_source_order` reads `map-tasks.yaml` and asserts the deserialized `IndexMap` keys equal the source order `[build, lint, test]`, and `map_form_preserves_unsorted_order_pinning_indexmap` proves the unsorted `[zebra, alpha, mango]` survives (`assert_ne!` against the sorted order rules out a BTreeMap). Reviewer can observe source key order preserved.

## Regression check

- No existing callers in scope — greenfield; nothing to regress.

## Residue

- The crate is deserialize-only; the O4 "re-serialize and diff" may require a test-only `Serialize` derive or a structural key-order assertion in place of a full round-trip re-emit — either satisfies the intent (key order preserved), a `Deserialize`-only regression does not.
- `folder-layout/` and `standalone/` are multi-file fixtures — confirm the round-trip test resolves the intended entry artifact rather than skipping the directory.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE — every obligation O1…O4 is SATISFIED, and the regression check holds (greenfield; whole workspace clippy + 13 nextest tests green, so no downstream breakage).
CONFIDENCE: high
SUMMARY: The deserialize-only mirror covers all 28 `tmx.schema.json` `$defs` field-for-field (closed↔`deny_unknown_fields`, open↔flatten `extra`, required↔non-`Option`); the whole 24-file example corpus deserializes without loss and exercises all ten `TaskWith` variants; array- and map-form task lists preserve source order (map form pinned to `indexmap` by an unsorted-key witness); a mismatched `with`/`type` is rejected at deserialize; and fmt/clippy/nextest/purity are all clean.
