# Task 21 — Filesystem adapter (`file`)

**Plan:** [plan.md](../plan.md) · **Certificate:** [21-filesystem_adapter-certificate.md](21-filesystem_adapter-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Executor ports (`FileSystem`)
**Depends on:** 05, 17
**Produces:** `LocalFileSystem` — the `file` executor covering read/write/append/delete/copy/move/exists with `encoding`
**Pointers:** `crates/tmx-adapters/src/fs.rs` (new), `crates/tmx-cli/src/compose.rs` (wire into the bundle)

## Steps

- [x] Implement the `FileSystem` port over the `file` operations: read, write, append, delete, copy, move, exists, honouring the `encoding` field.
- [x] Bound a read's captured output by `CAPTURED_OUTPUT_MAX_BYTES`, and translate an I/O error into a typed `RunError` — a missing path or permission failure is never a panic.
- [x] Wire the adapter into the composition root, behind its Cargo feature.
- [x] Add tests for each operation plus a missing-path read and an over-cap read.

## Definition of done

- [x] Each `file` operation executes against the local filesystem with the requested encoding, and `exists` reports correctly for present and absent paths.
- [x] A missing-path read is a typed `RunError` and an over-cap read returns `output_too_large` (negative space), not a panic.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run a flow chaining write → read → move → exists and confirm the state reflects each step, then confirm the missing-path error.
