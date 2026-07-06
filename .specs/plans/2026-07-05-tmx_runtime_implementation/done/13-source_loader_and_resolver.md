# Task 13 — Source loader and reference resolver adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** [13-source_loader_and_resolver-certificate.md](13-source_loader_and_resolver-certificate.md)

**Implements:** [03-loading-and-preflight.md](../../../03-loading-and-preflight.md) §Source loading and `kind` dispatch, §Reference resolution; [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Cross-cutting driven ports (`SourceLoader`, `ReferenceResolver`)
**Depends on:** 05
**Produces:** the `SourceLoader` (YAML/JSON/JSONC/TOML → one identical JSON model, `kind`-dispatched) and the file-path `ReferenceResolver` with cyclic-import detection
**Pointers:** `crates/tmx-adapters/src/loader/` (new, per-format modules + kind dispatch), `crates/tmx-adapters/src/resolve.rs` (new)

## Steps

- [x] Implement one loader per format (YAML, JSON, JSONC, TOML) producing the identical `serde_json::Value` model, feeding keys to an order-preserving map so the task map form keeps source order.
- [x] Implement `kind` dispatch (explicit `kind`, else the reserved `environment.*`/`context.*` filename convention, else "a top-level doc with `tasks` is a Flow"), mapping to the schema target.
- [x] Implement the file-path `ReferenceResolver`: resolve relative to the referring document's directory, load and `kind`-dispatch the target, and track the resolution chain to detect a cyclic `flow` import, returning `ResolutionError` rather than recursing.
- [x] Add a cross-format parity test: the same logical Flow in all four formats yields byte-identical models.

## Definition of done

- [x] All four formats parse to one identical model on the example corpus, `kind` dispatch selects the right target, and a reference resolves relative to its referrer.
- [x] A cyclic `flow` import returns `ResolutionError` instead of recursing forever (negative space), and an unknown extension or unreadable path is a typed error.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the cross-format parity test and the cyclic-import test, and load a reference chain to confirm resolution is relative to the referrer.
