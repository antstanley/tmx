# Task 01 — Workspace scaffold and quality gates

**Plan:** [plan.md](../plan.md) · **Certificate:** [01-workspace_scaffold-certificate.md](01-workspace_scaffold-certificate.md)

**Implements:** [02-crate-architecture.md](../../../02-crate-architecture.md) §Workspace layout, §Dependency graph, §Async model; [architecture-principles.md](../../../architecture-principles.md) §3 Rust conventions, §4 Composition root; [development-guidelines.md](../../../development-guidelines.md) §Toolchain, §Rust conventions (Formatting and linting), §Version control
**Depends on:** —
**Produces:** the five-crate Cargo workspace that builds clean under the full lint/format gate, with a `cargo tree`-based purity check that rejects an I/O or async dependency reaching `tmx-core`/`tmx-schema`/`tmx-testkit`
**Pointers:** `Cargo.toml` (workspace root, new), `rust-toolchain.toml` (new), `rustfmt.toml` (new), `clippy.toml` (new), `crates/tmx-{schema,core,adapters,testkit,cli}/` (new), `scripts/validate.sh` and `scripts/push.sh` (extend the gate), `.githooks/pre-push`

## Steps

- [ ] Create the workspace `Cargo.toml`: `[workspace]` members for the five crates, `resolver = "2"`, shared `[workspace.lints]` (`unsafe_code = "forbid"`, `clippy::unwrap_used`/`expect_used` denied outside tests, `unused_must_use` denied), pinned shared dependency versions, and `rust-version = "1.96.1"`.
- [ ] Add `rust-toolchain.toml` pinning the stable channel with `rustfmt`/`clippy` components and edition 2024; `rustfmt.toml` with `max_width = 100`; `clippy.toml` whose every opt-out carries an explanatory comment.
- [ ] Scaffold the five crates with their `lib.rs`/`main.rs`, each carrying `#![forbid(unsafe_code)]` and a crate-level doc comment naming what it is and the ports it depends on; wire the `tmx-cli → tmx-adapters → tmx-core → tmx-schema` and `tmx-testkit → tmx-core + tmx-schema` edges only.
- [ ] Add the `cargo tree`-based purity check (a script invoked by CI and `scripts/validate.sh`) asserting `tmx-core`/`tmx-schema`/`tmx-testkit` have no `tokio`/`reqwest`/S3-SDK/`std::process` edge in their dependency trees.
- [ ] Extend `scripts/push.sh`/CI so the pre-push gate runs `cargo fmt --check`, `cargo clippy --all-targets --all-features -D warnings`, `cargo nextest run`, and the purity check alongside the existing schema validation.

## Definition of done

- [ ] `cargo build`, `cargo fmt --all --check`, and `cargo clippy --all-targets --all-features -D warnings` all pass on the empty workspace, and the crates build against MSRV 1.96.1.
- [ ] The purity check **fails** when a `tokio` dependency is temporarily added to `tmx-core` (negative space: the guard actually guards) and passes once removed.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `scripts/push.sh`'s gate locally and watch fmt, clippy, the empty-crate build, and the purity check all pass, then confirm the purity check trips on an injected I/O edge.
