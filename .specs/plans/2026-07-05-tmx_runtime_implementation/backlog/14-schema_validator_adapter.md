# Task 14 — Schema validator adapter (JSON Schema 2020-12)

**Plan:** [plan.md](../plan.md) · **Certificate:** [14-schema_validator_adapter-certificate.md](14-schema_validator_adapter-certificate.md)

**Implements:** [03-loading-and-preflight.md](../../../03-loading-and-preflight.md) §Validation; [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Cross-cutting driven ports (`SchemaValidator`); [development-guidelines.md](../../../development-guidelines.md) §Defensive coding (Source file → loader boundary)
**Depends on:** 05
**Produces:** the `SchemaValidator` adapter validating artifacts and `produces` against the data-model schema (Draft 2020-12), `kind`-dispatched, at parity with `scripts/validate.sh`
**Pointers:** `crates/tmx-adapters/src/validate.rs` (new), `docs/tmx.schema.json`, `docs/tmx-provider.schema.json`, `scripts/validate.sh`

## Steps

- [ ] Select a JSON-Schema-2020-12 validator crate covering `$ref`, `allOf`/`if`/`then`, and cross-file `$ref` (for the provider manifest); if the feature set is insufficient, record it as an Open question.
- [ ] Implement `SchemaValidator` validating an artifact against the schema for its `kind`, returning one or more `Diagnostic`s and a `Validation` error on failure.
- [ ] Expose a mode that validates a task's `produces` schema against an output value, for the runtime `produces` check and `lint`.
- [ ] Add a parity test asserting the adapter agrees with `scripts/validate.sh` over the whole example corpus (accept the valid, reject the invalid).

## Definition of done

- [ ] Every valid corpus artifact validates and every intentionally invalid fixture is rejected with a `Diagnostic`, matching `scripts/validate.sh`'s verdicts.
- [ ] A cross-file `$ref` into the provider manifest resolves, and a malformed artifact yields a `Validation` error (not a panic) naming the failing path (negative space).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the corpus parity test and confirm the adapter and `scripts/validate.sh` reach identical verdicts.
