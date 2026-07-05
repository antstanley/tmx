# Done Certificate — Task 13: Source loader and reference resolver adapters

**Task:** [13-source_loader_and_resolver.md](13-source_loader_and_resolver.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 13. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 13) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Deliver the `SourceLoader` (YAML/JSON/JSONC/TOML → one identical JSON model, `kind`-dispatched) and the file-path `ReferenceResolver` with cyclic-import detection.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-05 driven-port traits `SourceLoader` and `ReferenceResolver` are implemented as declared, not modified; the Task-01 workspace purity/lint gates stay green (no tokio edge into `tmx-core`/`tmx-schema`/`tmx-testkit`).

## Obligations

- **O1 — All four formats parse to one identical model, `kind` dispatch selects the right target, and a reference resolves relative to its referrer.**
  - *Claim:* The YAML/JSON/JSONC/TOML loaders each produce a byte-identical `serde_json::Value` model for the same logical Flow; `kind` dispatch (explicit `kind`, else the `environment.*`/`context.*` filename convention, else "a top-level doc with `tasks` is a Flow") maps each artifact to the right schema target; a relative reference resolves against the referring document's directory.
  - *Evidence to collect:* Read `crates/tmx-adapters/src/loader/` (per-format modules + kind dispatch) and `crates/tmx-adapters/src/resolve.rs`. Run the cross-format parity test over `docs/examples/single-file-flow.{yaml,json,jsonc,toml}` and expect one identical model across all four. Load the `kind`-dispatched corpus artifacts and confirm each maps to the right target.
  - *Checks:* Resolve the task map-form key insertion to the order-preserving `IndexMap`, not a `HashMap`, so the task map form keeps source order; resolve a relative reference against the referring document's directory, not the process cwd.
  - *Status:* ☐ unverified

- **O2 — A cyclic `flow` import returns `ResolutionError` instead of recursing forever, and an unknown extension or unreadable path is a typed error.**
  - *Claim:* The resolver tracks the resolution chain and returns `ResolutionError` when a `flow` import re-enters an ancestor, rather than recursing unbounded; an unknown file extension and an unreadable path each surface as a typed error, not a panic.
  - *Evidence to collect:* Read the resolution-chain tracking in `crates/tmx-adapters/src/resolve.rs`. Run the cyclic-import test and expect a `ResolutionError` (test terminates, no stack overflow). Run the unknown-extension and unreadable-path tests and expect typed errors, not panics.
  - *Checks:* Trace the resolution-chain tracking so a `flow` that imports a document already in its chain returns `ResolutionError` before re-invoking the loader on it.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant, and the adapter stays at corpus parity with `scripts/validate.sh`.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run `scripts/validate.sh` and expect it clean over the corpus the loader consumes (`docs/examples/`). Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Run the cross-format parity test and the cyclic-import test, and load a reference chain to confirm resolution is relative to the referrer (Reviewable).**
  - *Claim:* A reviewer can run the two named tests and observe pass, and load a multi-document reference chain and observe each reference resolving against its referrer's directory.
  - *Evidence to collect:* Run the parity test (`docs/examples/single-file-flow.*`) and the cyclic-import test via `cargo nextest run`; observe pass. Load a reference chain (a Flow importing a sibling document) and observe the referenced path resolved relative to the referring file, not the cwd.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The composition root that will consume `SourceLoader`/`ReferenceResolver` is Task 17.

## Residue

The parity test asserts byte-identical models across formats; JSONC comment stripping and TOML value typing (integers vs strings, tables vs maps) are the likely divergence points to spot-check. The `environment.*`/`context.*` filename convention is same-folder only. Verify the map-form order preservation with a fixture whose keys are not already alphabetical.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
