# Done Certificate — Task 05: Core port traits (driven and driving)

**Task:** [05-core_ports.md](05-core_ports.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 05. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 05) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** The port traits the core owns — every driven capability and every driving use case — with the pure/async boundary fixed at the trait layer.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** Task 04's `tmx-core` runtime model and `RunError`. The port traits reference `RunError` as their uniform `Result` error type and the runtime entities as request/response payloads; that surface must remain intact. No adapter or runner consumes the ports yet — the consumer is not built until Tasks 06/11+.

## Obligations

- **O1 — Every port named in `06-ports-and-adapters.md` exists as a trait in `ports::driven`, and every command's use case in `07-cli.md` has a trait in `ports::driving`.**
  - *Claim:* `ports::driven` declares all fifteen driven port traits and `ports::driving` declares all ten driving use-case traits named in the spec.
  - *Evidence to collect:* read the planned `crates/tmx-core/src/ports/driven.rs` and `driving.rs`. Against [`06-ports-and-adapters.md`](../../../06-ports-and-adapters.md) confirm driven traits `ProcessRunner`, `HttpClient`, `FileSystem`, `ObjectStore`, `ChatModel`, `SecretResolver`, `EnvironmentProvider`, `RunStore`, `EventSink`, `SourceLoader`, `ReferenceResolver`, `SchemaValidator`, `Clock`, `IdGenerator`, `Scheduler`. Against [`07-cli.md#command--use-case-mapping`](../../../07-cli.md#command--use-case-mapping) confirm driving traits `RunFlow`, `ValidateArtifacts`, `LintFlow`, `InspectFlow`, `ScaffoldFlow`, `FormatArtifact`, `Discover`, `ProvisionEnvironment`, `ManageProviders`, `QueryRuns`. Confirm each driven method returns `Result<_, RunError>` and is async only at the effecting boundary, and that the sketched request/response types (`ProcessOutput`, `HttpResponse`, `FileOp`/`FileResult`, `StoreOp`/`StoreResult`, `ChatRequest`/`ChatResponse`) live in the core.
  - *Checks:* resolve that each port method's error type is `tmx-core`'s `RunError` from Task 04, not a re-declared local error or a `std::io::Error`.
  - *Status:* ☐ unverified

- **O2 — Negative space: the traits compile with the core still I/O-free (the `cargo tree` purity gate stays green), and a compile check confirms the driven ports are usable as `dyn`.**
  - *Claim:* the traits compile, `tmx-core` stays I/O-free, and driven ports are object-safe where the composition root needs `dyn`.
  - *Evidence to collect:* run `cargo build -p tmx-core` and expect success; run the `cargo tree` purity check and confirm `tmx-core` has no async runtime or I/O edge. Confirm a compile check (e.g. a `fn _assert(_: &dyn ProcessRunner)` or a `Box<dyn Port>` construction in a test/doctest) demonstrates `dyn`-usability for the ports the composition root injects; confirm `#[async_trait]` is used only where native `async fn` breaks object safety, and port handles are `#[must_use]`.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-core` stays free of any async runtime or I/O crate. Task 05 declares traits only and changes no schema/example, so `scripts/validate.sh` is not required by this task.
  - *Status:* ☐ unverified

- **O4 — Reviewable: read `ports/driven.rs` and `ports/driving.rs` against the port and use-case tables in the spec, and confirm `cargo build` keeps `tmx-core` free of any I/O crate.**
  - *Claim:* a reviewer can diff `ports/driven.rs` and `ports/driving.rs` against the port and use-case tables and observe `cargo build` leave `tmx-core` I/O-free.
  - *Evidence to collect:* open `crates/tmx-core/src/ports/driven.rs` and `driving.rs` beside the [06 port table](../../../06-ports-and-adapters.md) and the [07 use-case table](../../../07-cli.md#command--use-case-mapping) and confirm 1:1 coverage; run `cargo build` and the `cargo tree` purity check and observe `tmx-core` pulls in no I/O crate.
  - *Status:* ☐ unverified

## Regression check

- Task 05 builds on the Task 04 `tmx-core` model. Trace that `RunError` and the runtime entities (`TaskResult`, `Event`, `RunRecord`, …) remain the referenced error/payload types across the port signatures, and that Task 04's serialize-and-validate test still passes unchanged : ☐ (PRESERVED / REGRESSION). The port consumer (fakes, runner) is not yet built.

## Residue

- The async-mechanism open question (native `async fn` in traits vs `#[async_trait]`) is settled per port inside this task — confirm the choice is recorded per port and that object-safety is actually exercised by a compile check, not assumed.
- `ports/mod.rs` should re-export both submodules; confirm the module wiring compiles as declared in the Pointers.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
