# Task 05 — Core port traits (driven and driving)

**Plan:** [plan.md](../plan.md) · **Certificate:** [05-core_ports-certificate.md](05-core_ports-certificate.md)

**Implements:** [02-crate-architecture.md](../../../02-crate-architecture.md) §tmx-core (`ports/`); [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Executor ports, §Cross-cutting driven ports (the traits); [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §The Scheduler (the trait); [architecture-principles.md](../../../architecture-principles.md) §1 Hexagonal layering, §3 Rust conventions (async in traits)
**Depends on:** 04
**Produces:** the port traits the core owns — every driven capability and every driving use case — with the pure/async boundary fixed at the trait layer
**Pointers:** `crates/tmx-core/src/ports/mod.rs` (new), `crates/tmx-core/src/ports/driven.rs` (new), `crates/tmx-core/src/ports/driving.rs` (new)

## Steps

- [x] Declare the driven port traits: `ProcessRunner`, `HttpClient`, `FileSystem`, `ObjectStore`, `ChatModel`, `SecretResolver`, `EnvironmentProvider`, `RunStore`, `EventSink`, `SourceLoader`, `ReferenceResolver`, `SchemaValidator`, `Clock`, `IdGenerator`, and `Scheduler`, each method returning `Result<_, RunError>` and each async at the effecting boundary only.
- [x] Declare the driving use-case traits: `RunFlow`, `ValidateArtifacts`, `LintFlow`, `InspectFlow`, `ScaffoldFlow`, `FormatArtifact`, `Discover`, `ProvisionEnvironment`, `ManageProviders`, `QueryRuns`.
- [x] Choose native `async fn` in traits where object safety allows and `#[async_trait]` only where `dyn Port` forces it (settling the [open question](../../../architecture-principles.md#assumptions-and-open-questions) per port), and mark port handles `#[must_use]`.
- [x] Sketch the associated request/response types the traits reference (`ProcessOutput`, `HttpResponse`, `FileOp`/`FileResult`, `StoreOp`/`StoreResult`, `ChatRequest`/`ChatResponse`), keeping them in the core.

## Definition of done

- [x] Every port named in [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) exists as a trait in `ports::driven`, and every command's use case in [07-cli.md](../../../07-cli.md#command--use-case-mapping) has a trait in `ports::driving`.
- [x] Negative space: the traits compile with the core still I/O-free — the `cargo tree` purity gate stays green (no I/O or async edge reaches `tmx-core`) — and a compile check confirms the driven ports are usable as `dyn` where the composition root needs them.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: read `ports/driven.rs` and `ports/driving.rs` against the port and use-case tables in the spec, and confirm `cargo build` keeps `tmx-core` free of any I/O crate.
