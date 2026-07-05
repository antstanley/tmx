# Done Certificate — Task 01: Workspace scaffold and quality gates

**Task:** [01-workspace_scaffold.md](01-workspace_scaffold.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 01. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 01) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The five-crate Cargo workspace builds clean under the full lint/format gate, and a `cargo tree`-based purity check rejects an I/O or async dependency reaching `tmx-core`/`tmx-schema`/`tmx-testkit`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** None — greenfield foundation; no prior behavior to preserve.

## Obligations

- **O1 — The empty workspace builds, formats, and lints clean, and the crates build against MSRV 1.96.1.**
  - *Claim:* the five-crate workspace compiles with zero warnings on empty crates, passes `fmt` and `clippy`, and builds under Rust 1.96.1.
  - *Evidence to collect:* from the repo root run `cargo build`, `cargo fmt --all --check`, and `cargo clippy --all-targets --all-features -D warnings` — expect all clean. Confirm the planned `rust-toolchain.toml` and the workspace `Cargo.toml` `rust-version = "1.96.1"` pin the MSRV; build once under the 1.96.1 toolchain (e.g. `cargo +1.96.1 build`) and expect success. Confirm the five members `crates/tmx-{schema,core,adapters,testkit,cli}/` exist, each with `lib.rs`/`main.rs` carrying `#![forbid(unsafe_code)]` and a crate-level doc comment, and that only the `tmx-cli → tmx-adapters → tmx-core → tmx-schema` and `tmx-testkit → tmx-core + tmx-schema` edges are wired.
  - *Status:* ☐ unverified

- **O2 — The purity check fails on an injected `tokio` edge into `tmx-core` and passes once removed.**
  - *Claim:* the `cargo tree` purity guard actually guards — adding a `tokio` dependency to `tmx-core` makes the check fail; removing it makes it pass.
  - *Evidence to collect:* temporarily add a `tokio` dependency to `crates/tmx-core/Cargo.toml`, run the purity check (the `cargo tree` script invoked by `scripts/validate.sh`/CI), and expect a non-zero exit naming the offending edge; remove the dependency and expect the check to pass.
  - *Checks:* confirm the purity script inspects the `tmx-core`, `tmx-schema`, and `tmx-testkit` trees for `tokio`/`reqwest`/S3-SDK/`std::process` edges — not `tmx-core` alone.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean (the empty workspace carries no tests yet, so `nextest` is expected to report zero failures). Task 01 introduces no runtime bound constant (those land in Task 02); confirm no limit value is hard-coded in the scaffold.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run `scripts/push.sh`'s gate locally and watch fmt, clippy, the empty-crate build, and the purity check all pass, then confirm the purity check trips on an injected I/O edge.**
  - *Claim:* a reviewer can run the pre-push gate locally and observe `fmt`, `clippy`, the empty-crate build, and the purity check all pass, then observe the purity check trip when an I/O edge is injected.
  - *Evidence to collect:* run the planned/extended `scripts/push.sh` at the repo root and observe `fmt`, `clippy`, `cargo build` over the five empty crates, and the purity check all report success. Then inject a `tokio` (or `std::process`) edge into `tmx-core`, re-run, observe the purity check fail, and revert.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress.

## Residue

- The `.githooks/pre-push` and `scripts/push.sh` extensions are gate wiring, not behavior — confirm the gate is actually invoked by the hook, not only runnable by hand.
- MSRV verification requires the 1.96.1 toolchain installed; if unavailable, record the pin as read from config rather than a live build.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
