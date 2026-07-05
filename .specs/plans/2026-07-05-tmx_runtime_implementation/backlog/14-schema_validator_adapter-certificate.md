# Done Certificate — Task 14: Schema validator adapter (JSON Schema 2020-12)

**Task:** [14-schema_validator_adapter.md](14-schema_validator_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

- **O2 — A cross-file `$ref` into the provider manifest resolves, and a malformed artifact yields a `Validation` error (not a panic) naming the failing path.**
  - *Claim:* A cross-file `$ref` from an artifact into the provider manifest resolves and validates; a malformed artifact produces a `Validation` error naming the failing JSON path, never a panic.
  - *Evidence to collect:* Read the `$ref` resolution and error path in `crates/tmx-adapters/src/validate.rs`. Run the cross-file-`$ref` test that reaches into `docs/tmx-provider.schema.json` and expect it to resolve. Run the malformed-artifact test and expect a `Validation` error naming the failing path (assert no panic).
  - *Checks:* Resolve the cross-file `$ref` from the provider manifest to `docs/tmx-provider.schema.json`, confirming the referenced schema loads rather than erroring.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant, and the adapter stays at corpus parity with `scripts/validate.sh`.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run `scripts/validate.sh` and expect it clean over `docs/examples/`. Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Run the corpus parity test and confirm the adapter and `scripts/validate.sh` reach identical verdicts (Reviewable).**
  - *Claim:* A reviewer can run the parity test and observe the adapter's verdicts identical to `scripts/validate.sh`'s over the whole corpus.
  - *Evidence to collect:* Run the corpus parity test via `cargo nextest run`; run `scripts/validate.sh`; observe identical accept/reject verdicts artifact by artifact.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The preflight (Task 15) and the composition root (Task 17) that will consume `SchemaValidator` come later.

## Residue

The 2020-12 validator crate choice is a Step deliverable; if its coverage (`$ref`, `allOf`/`if`/`then`, cross-file `$ref`) proves insufficient, the task records it as an Open question — check the task file for that note. Parity is verdict-level (accept/reject), not diagnostic-text-level; do not require identical message strings. The `produces`-mode validation (a task's `produces` schema against an output value) is a Step — confirm it exists even though the DoD phrases parity in artifact terms.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
