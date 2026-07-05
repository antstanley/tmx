# Done Certificate — Task 05: Core port traits (driven and driving)

**Task:** [05-core_ports.md](05-core_ports.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

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
  - *Status:* ☑ SATISFIED — `driven.rs` declares all 15 named driven traits (ProcessRunner, HttpClient, FileSystem, ObjectStore, ChatModel — the five executor ports of 06 §Executor ports; SourceLoader, ReferenceResolver, SchemaValidator, SecretResolver, EnvironmentProvider, RunStore, EventSink, Clock, IdGenerator, Scheduler — the ten cross-cutting ports of 06 §Cross-cutting driven ports). `driving.rs` declares all 10 use-case traits 1:1 with the 07 §Command→use-case table (RunFlow, ValidateArtifacts, LintFlow, InspectFlow, ScaffoldFlow, FormatArtifact, Discover, ProvisionEnvironment, ManageProviders, QueryRuns); `version`/`help` correctly have no use case, and InspectFlow covers `inspect`/`context show`/`secrets list` per the table. Every fallible method returns `Result<_, RunError>` using Task-04's `RunError` (grep confirms no `std::io::Error`, no local error type). The sketched DTOs (ProcessSpec/ProcessOutput, HttpRequest/HttpResponse, FileOp/FileResult, StoreOp/StoreResult, ChatRequest/ChatResponse, and the driving DTOs) all live in `tmx-core::ports`. `Clock::now`/`Clock::now_ms`/`IdGenerator::new_run_id` return bare values, not `Result` — an infallible read, mandated by development-guidelines "Simpler return types win: `()` > … > `Result<T,E>`" and its ban on dead error paths; not a deviation but repo-DoD-compliant.

- **O2 — Negative space: the traits compile with the core still I/O-free (the `cargo tree` purity gate stays green), and a compile check confirms the driven ports are usable as `dyn`.**
  - *Claim:* the traits compile, `tmx-core` stays I/O-free, and driven ports are object-safe where the composition root needs `dyn`.
  - *Evidence to collect:* run `cargo build -p tmx-core` and expect success; run the `cargo tree` purity check and confirm `tmx-core` has no async runtime or I/O edge. Confirm a compile check (e.g. a `fn _assert(_: &dyn ProcessRunner)` or a `Box<dyn Port>` construction in a test/doctest) demonstrates `dyn`-usability for the ports the composition root injects; confirm `#[async_trait]` is used only where native `async fn` breaks object safety, and port handles are `#[must_use]`.
  - *Status:* ☑ SATISFIED — `cargo build --tests -p tmx-core` succeeds; `scripts/purity.sh` PASSES ("tmx-schema, tmx-core, tmx-testkit carry no I/O or async dependency edge"), and `scripts/purity_selftest.sh` PASSES — the guard trips on an injected forbidden edge, so the green result is real. The 14 dyn-usable driven ports + all 10 use cases are bound as `Box<dyn Port>` in `every_driven_port_is_object_safe_as_dyn` / `every_driving_use_case_is_object_safe_as_dyn` (both pass); I independently exercised this guard by injecting a generic method into `Clock` and observed E0038 "the trait `Clock` is not dyn compatible" at `mod.rs:443` (`Box<dyn Clock>`), then reverted — so the compile check is a real negative-space guard, not a decoration. `#[async_trait]` is applied only to the effecting ports that are held as `dyn`; the three sync ports and the generic `Scheduler` (which is object-unsafe by design — generic method, used behind a generic bound, exercised by `SerialScheduler`) carry no macro. Every port trait carries `#[must_use = "…port handle…"]`.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-core` stays free of any async runtime or I/O crate. Task 05 declares traits only and changes no schema/example, so `scripts/validate.sh` is not required by this task.
  - *Status:* ☑ SATISFIED — I ran all four from the repo root: `cargo fmt --all --check` clean (exit 0); `cargo clippy --all-targets --all-features -- -D warnings` clean (exit 0, no warnings, `tmx-core` re-checked fresh); `cargo nextest run` → 31 tests run, 31 passed, 0 skipped, including all 8 new `ports::tests` (object-safety ×2, boxed RunFlow terminal record, async-route-via-dyn, failing-port-typed-RunError, sync-ports, RunStore/EventSink model types, Scheduler bounded/index-ordered); `scripts/purity.sh` PASS. Named-constant rule: this task adds no numeric bound — DTO docs reference the existing `tmx-schema::limits` constants (`CAPTURED_OUTPUT_MAX_BYTES`, `EVENT_LOG_MAX_BYTES`, `CONCURRENCY_MAX`, `FANOUT_WIDTH_MAX`, all confirmed to exist); no magic number introduced. No schema/example changed, so `scripts/validate.sh` is correctly not required.

- **O4 — Reviewable: read `ports/driven.rs` and `ports/driving.rs` against the port and use-case tables in the spec, and confirm `cargo build` keeps `tmx-core` free of any I/O crate.**
  - *Claim:* a reviewer can diff `ports/driven.rs` and `ports/driving.rs` against the port and use-case tables and observe `cargo build` leave `tmx-core` I/O-free.
  - *Evidence to collect:* open `crates/tmx-core/src/ports/driven.rs` and `driving.rs` beside the [06 port table](../../../06-ports-and-adapters.md) and the [07 use-case table](../../../07-cli.md#command--use-case-mapping) and confirm 1:1 coverage; run `cargo build` and the `cargo tree` purity check and observe `tmx-core` pulls in no I/O crate.
  - *Status:* ☑ SATISFIED — read `driven.rs` (532 lines) and `driving.rs` (202 lines) line-by-line against the 06 port table and the 07 use-case table: 15/15 driven ports and 10/10 use cases present with matching sub-vocabularies (FileOp read/write/append/delete/copy/move/exists; StoreOp get/put/delete/list/head; SourceKind yaml/json/jsonc/toml; ProviderMethod bootstrap/deploy/clean/destroy — all match the spec). `cargo build` succeeds; `scripts/purity.sh` confirms `tmx-core`'s normal-edge tree carries no I/O/async crate (only `async-trait`, a compile-time proc-macro that 02 §tmx-core permits as a "trait-support" crate and whose build deps are pure).

## Regression check

- Task 05 builds on the Task 04 `tmx-core` model. Trace that `RunError` and the runtime entities (`TaskResult`, `Event`, `RunRecord`, …) remain the referenced error/payload types across the port signatures, and that Task 04's serialize-and-validate test still passes unchanged : ☑ PRESERVED — `model.rs` and `error.rs` are untouched (not in the diff); the ports reference `RunError`, `RunRecord`, `Event`, `RunId`, `Diagnostic`, `Milliseconds`, `Timestamp` from Task 04 directly, and the Task-04 canonical-types integration tests (`every_runtime_def_has_a_type_that_validates`, `the_validator_rejects_out_of_contract_shapes`) plus the model/error unit tests all pass unchanged in the 31/31 run. The port consumer (fakes, runner) is not yet built, as expected.

## Residue

- The async-mechanism open question (native `async fn` in traits vs `#[async_trait]`) is settled per port inside this task — confirm the choice is recorded per port and that object-safety is actually exercised by a compile check, not assumed.
- `ports/mod.rs` should re-export both submodules; confirm the module wiring compiles as declared in the Pointers.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☑ DONE — every obligation O1…O4 is SATISFIED and the regression check is PRESERVED.
CONFIDENCE: ☑ high — all four gate commands and both purity scripts were run first-hand with clean results; the object-safety negative space was exercised by injection (E0038 observed, then reverted); all 25 named traits and every DTO were read against the 06/07 spec tables.
SUMMARY: Task 05 lands all 15 driven and 10 driving port traits in `tmx-core::ports`, 1:1 with the 06/07 spec tables, uniformly typed on `RunError`, with the pure/async boundary fixed at the trait layer (effecting ports `#[async_trait]` for `dyn`, sync ports and the generic `Scheduler` native) and every handle `#[must_use]`. fmt/clippy(-D warnings)/nextest(31/31, 8 new)/purity all clean; the core stays I/O-free; Task-04 is untouched. Residue settled: the async-mechanism open question is resolved per-port and object safety is proven by a guard I confirmed genuinely trips.
