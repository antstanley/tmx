# Done Certificate — Task 13: Source loader and reference resolver adapters

**Task:** [13-source_loader_and_resolver.md](13-source_loader_and_resolver.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

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
  - *Status:* ☑ SATISFIED — `all_four_formats_parse_to_one_identical_model` passes over `docs/examples/single-file-flow.{yaml,json,jsonc,toml}`, spot-checking the residue's TOML-integer (`200`) and string-`timeout` (`"5m"`) divergence points; `kind_dispatch_selects_the_right_target_across_the_corpus` maps Flow/Provider/Environment/Context/Task/Task across the corpus per the 03 dispatch table; `a_reference_resolves_relative_to_its_referrer_not_the_cwd` passes with the sibling present only in the referrer's dir. Map-form order verified with a non-alphabetical fixture (`zebra/mango/alpha`) in `the_task_map_form_keeps_source_key_order`; mutation check: removing the `preserve_order` feature made that test FAIL (then reverted, suite green) — the order-preserving `IndexMap` is load-bearing, not vacuously asserted.

- **O2 — A cyclic `flow` import returns `ResolutionError` instead of recursing forever, and an unknown extension or unreadable path is a typed error.**
  - *Claim:* The resolver tracks the resolution chain and returns `ResolutionError` when a `flow` import re-enters an ancestor, rather than recursing unbounded; an unknown file extension and an unreadable path each surface as a typed error, not a panic.
  - *Evidence to collect:* Read the resolution-chain tracking in `crates/tmx-adapters/src/resolve.rs`. Run the cyclic-import test and expect a `ResolutionError` (test terminates, no stack overflow). Run the unknown-extension and unreadable-path tests and expect typed errors, not panics.
  - *Checks:* Trace the resolution-chain tracking so a `flow` that imports a document already in its chain returns `ResolutionError` before re-invoking the loader on it.
  - *Status:* ☑ SATISFIED — `walk_flow_imports` (resolve.rs:117) checks `chain.contains(&path)` on the canonicalized path BEFORE calling `loader.load`, returning typed `cyclic_flow_import` with the full trace; `chain.pop()` after children means a diamond is not a false positive. `a_cyclic_flow_import_returns_a_resolution_error_instead_of_recursing` (a↔b) terminates with `cyclic_flow_import` / Resolution category; `an_acyclic_import_chain_resolves_relative_to_each_referrer` (a→b→c) passes. `an_unreadable_path_and_an_unknown_extension_are_distinct_typed_errors` shows `reference_not_found` vs `unknown_source_format` as distinct typed Resolution errors; `detect_source_kind` rejects a missing extension; the JSONC malformed-residue test yields typed `source_parse_error`, no panics anywhere.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant, and the adapter stays at corpus parity with `scripts/validate.sh`.
  - *Evidence to collect:* Run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run `scripts/validate.sh` and expect it clean over the corpus the loader consumes (`docs/examples/`). Confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — validator ran all four independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 135/135 passed (18 new adapter tests, no regression in the prior 117); `scripts/validate.sh` all checks passed including cross-format corpus parity; `scripts/purity.sh` green (no async/I-O edge into the pure crates — the loader reads via sync `std::fs`). No new numeric bound introduced (cycle detection is chain-membership; `FLOW_DEPTH_MAX` remains the documented execution-time backstop), so nothing was owed to `tmx-schema::limits`.

- **O4 — Run the cross-format parity test and the cyclic-import test, and load a reference chain to confirm resolution is relative to the referrer (Reviewable).**
  - *Claim:* A reviewer can run the two named tests and observe pass, and load a multi-document reference chain and observe each reference resolving against its referrer's directory.
  - *Evidence to collect:* Run the parity test (`docs/examples/single-file-flow.*`) and the cyclic-import test via `cargo nextest run`; observe pass. Load a reference chain (a Flow importing a sibling document) and observe the referenced path resolved relative to the referring file, not the cwd.
  - *Status:* ☑ SATISFIED — validator ran the five Reviewable tests by name (`cargo nextest run -p tmx-adapters -E 'test(...)'`): parity, cyclic-import, referrer-relative sibling, a→b→c chain, and map order — 5/5 passed. The chain test's fixtures exist only in a scratch dir (never the cwd), and each `use` resolves against its own file's directory via `FileReferenceResolver`, so the observed pass is direct evidence of referrer-relative resolution.

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The composition root that will consume `SourceLoader`/`ReferenceResolver` is Task 17.

## Residue

The parity test asserts byte-identical models across formats; JSONC comment stripping and TOML value typing (integers vs strings, tables vs maps) are the likely divergence points to spot-check. The `environment.*`/`context.*` filename convention is same-folder only. Verify the map-form order preservation with a fixture whose keys are not already alphabetical.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently executed evidence: four formats land in one identical `serde_json::Value` (corpus parity + `scripts/validate.sh`), `kind` dispatch follows the 03 precedence exactly (explicit `kind` → reserved filename → top-level `tasks` → task, unknown kind typed error), references resolve against the referrer's directory, and the chain-membership guard turns a cyclic `flow` import into a typed `cyclic_flow_import` before re-loading. fmt/clippy/nextest (135/135)/purity all green; a mutation check (dropping `preserve_order`) proved the map-order test is load-bearing. Greenfield — no downstream callers to regress (composition root arrives in Task 17).
