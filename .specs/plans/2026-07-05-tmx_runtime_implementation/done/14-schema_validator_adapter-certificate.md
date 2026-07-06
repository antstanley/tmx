# Done Certificate — Task 14: Schema validator adapter (JSON Schema 2020-12)

**Task:** [14-schema_validator_adapter.md](14-schema_validator_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

> This certificate is a verification protocol for Task 14. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 14) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Deliver the `SchemaValidator` adapter validating artifacts and `produces` against the data-model schema (Draft 2020-12), `kind`-dispatched, at parity with `scripts/validate.sh`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-05 driven-port trait `SchemaValidator` is implemented as declared, not modified; the Task-01 workspace purity/lint gates stay green.

## Obligations

- **O1 — Every valid corpus artifact validates and every intentionally invalid fixture is rejected with a `Diagnostic`, matching `scripts/validate.sh`'s verdicts.**
  - *Claim:* For each corpus artifact the adapter reaches the same accept/reject verdict as `scripts/validate.sh`; a rejection carries at least one `Diagnostic`.
  - *Evidence to collect:* Read `crates/tmx-adapters/src/validate.rs`. Run the corpus parity test over `docs/examples/` and expect every valid artifact accepted and every intentionally invalid fixture rejected with a `Diagnostic`. Cross-check the verdicts against `scripts/validate.sh`.
  - *Checks:* Resolve `kind` dispatch so each artifact is validated against the schema for its `kind` (Flow / Task / provider), not against a single catch-all schema.
  - *Status:* ☑ SATISFIED — `every_corpus_artifact_validates_at_parity_with_validate_sh` walks the same 24 files `scripts/validate.sh` reports (counts verified independently: 24 both sides), classifies each with the loader's `classify_artifact`, and accepts all with zero diagnostics; `scripts/validate.sh` independently reports 24/24 PASS. Dispatch is per-kind, proven by `each_kind_dispatches_to_its_own_schema_not_a_catch_all` (a Flow rejected by the Context schema; an environment rejected by the Flow schema). Rejection carries `Diagnostic`s: verified by the invalid-fixture tests and by injecting a bogus-task flow into `docs/examples/` — both the Rust parity test and `validate.sh` rejected it identically (injection reverted).

- **O2 — A cross-file `$ref` into the provider manifest resolves, and a malformed artifact yields a `Validation` error (not a panic) naming the failing path.**
  - *Claim:* A cross-file `$ref` from an artifact into the provider manifest resolves and validates; a malformed artifact produces a `Validation` error naming the failing JSON path, never a panic.
  - *Evidence to collect:* Read the `$ref` resolution and error path in `crates/tmx-adapters/src/validate.rs`. Run the cross-file-`$ref` test that reaches into `docs/tmx-provider.schema.json` and expect it to resolve. Run the malformed-artifact test and expect a `Validation` error naming the failing path (assert no panic).
  - *Checks:* Resolve the cross-file `$ref` from the provider manifest to `docs/tmx-provider.schema.json`, confirming the referenced schema loads rather than erroring.
  - *Status:* ☑ SATISFIED — ref topology verified from the schema sources: `tmx-provider.schema.json` `$ref`s `https://tmx.dev/schemas/0.2.0/tmx.schema.json#/$defs/task`, exactly the `$id` `MainSchemaRetriever` serves from the embedded main schema (any other URI errors loudly; the HTTP retriever is compiled out). `cross_file_ref_into_the_provider_manifest_resolves` passes both directions: the real manifest accepts, and a manifest with a bogus inline task `type` is rejected *through* the resolved `$ref`. `a_malformed_artifact_yields_a_diagnostic_naming_the_failing_path_not_a_panic` and `a_missing_required_field_reports_the_root_path` show every rejection is a `Diagnostic` with a JSON-pointer path (`/tasks/...`, root `/`) and no panic; the typed `Validation`-category `RunError` fault path is exercised by `validate_produces_faults_on_an_uncompilable_schema`. The spec-03 `ValidationError` wrapping of diagnostics is preflight's job (Task 15), consistent with the Task-05 port contract (`Diagnostic`s for rejection, `RunError` for internal faults).

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant, and the adapter stays at corpus parity with `scripts/validate.sh`.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run `scripts/validate.sh` and expect it clean over `docs/examples/`. Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — verifier ran all four independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -D warnings` clean; `cargo nextest run` 144/144 passed; `scripts/validate.sh` "all checks passed". `scripts/purity.sh` green (jsonschema is a `tmx-adapters`-only edge, `default-features = false`, no reqwest). No new numeric bound introduced, so `tmx-schema::limits` correctly untouched; the new string constants (`SCHEMA_VIOLATION_CODE`, `ROOT_POINTER`) are named. Every user-facing validation path has a negative-space test. Minor note (non-blocking): the `MainSchemaRetriever` unknown-URI branch — a defensive guard against an internal fault unreachable with the verified embedded-schema ref topology — has no direct unit test; it is not a user validation path.

- **O4 — Run the corpus parity test and confirm the adapter and `scripts/validate.sh` reach identical verdicts (Reviewable).**
  - *Claim:* A reviewer can run the parity test and observe the adapter's verdicts identical to `scripts/validate.sh`'s over the whole corpus.
  - *Evidence to collect:* Run the corpus parity test via `cargo nextest run`; run `scripts/validate.sh`; observe identical accept/reject verdicts artifact by artifact.
  - *Status:* ☑ SATISFIED — verifier ran `cargo nextest run -p tmx-adapters validate` (9/9 pass, parity test included) and `scripts/validate.sh` (24/24 artifacts PASS, listed per file); both accept all 24 corpus artifacts. Negative side exercised live: an injected invalid flow (`bogus-task-type`) made the parity test FAIL and `validate.sh` report `[FAIL] ... 'bogus-task-type' is not valid` — identical reject verdicts; the injection was reverted and the suite re-ran green.

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The preflight (Task 15) and the composition root (Task 17) that will consume `SchemaValidator` come later.

## Residue

The 2020-12 validator crate choice is a Step deliverable; if its coverage (`$ref`, `allOf`/`if`/`then`, cross-file `$ref`) proves insufficient, the task records it as an Open question — check the task file for that note. Parity is verdict-level (accept/reject), not diagnostic-text-level; do not require identical message strings. The `produces`-mode validation (a task's `produces` schema against an output value) is a Step — confirm it exists even though the DoD phrases parity in artifact terms.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED by independently-run evidence. The `JsonSchemaValidator` adapter implements the Task-05 `SchemaValidator` port unmodified, `kind`-dispatched over five compiled Draft-2020-12 validators from compile-time-embedded schemas; verdict parity with `scripts/validate.sh` verified over all 24 corpus artifacts on the accept side and by live invalid-fixture injection on the reject side (both tools rejected identically; injection reverted). Cross-file `$ref` resolution verified from the schema sources and by a two-direction test; every rejection is a path-bearing `Diagnostic`, internal faults are typed `Validation` `RunError`s, no panics. fmt/clippy/nextest (144/144)/validate.sh/purity all green; greenfield, no regressions. One non-blocking note: the defensive unknown-external-URI retriever branch lacks a direct unit test (the implementer's self-report overclaimed a test there), but it is an internal-fault guard, not a validation path, so the DoD's negative-space rule is met.
