# TMX Specifications

The formal implementation specification for the TMX Rust runtime and CLI, plus the plans that build it. Start at [`00-overview.md`](00-overview.md); the pages below are the canonical, repo-wide set.

## Canonical spec

| Page | Topic |
|---|---|
| [00-overview.md](00-overview.md) | Design overview: the hexagon, the crates, the scope, the decisions |
| [01-domain-model.md](01-domain-model.md) | Input and runtime entities as Rust types; ids; the Pipeline lifecycle |
| [02-crate-architecture.md](02-crate-architecture.md) | The Cargo workspace, crate boundaries, the dependency rule, the async model |
| [03-loading-and-preflight.md](03-loading-and-preflight.md) | Source loading, `kind` dispatch, reference resolution, directory assembly, preflight |
| [04-execution-engine.md](04-execution-engine.md) | The `PipelineRunner`, scopes and interpolation, secrets and masking, hooks, limits |
| [05-fan-out-and-eval.md](05-fan-out-and-eval.md) | `map` fan-out, `eval` measurement, the shared MatcherEngine, the Scheduler |
| [06-ports-and-adapters.md](06-ports-and-adapters.md) | Every driven port and its built-in adapter; the TaskDispatcher; providers |
| [07-cli.md](07-cli.md) | The `tmx` command surface, flags, output contract, configuration, exit codes |
| [08-errors-and-observability.md](08-errors-and-observability.md) | Error categories, the event stream, reporters, the run store, masking |
| [architecture-principles.md](architecture-principles.md) | Hexagonal layering + Tiger Style + Rust conventions (cross-cutting) |
| [development-guidelines.md](development-guidelines.md) | Toolchain, code style, limits, testing, the definition of done (cross-cutting) |
| [canonical-types.schema.json](canonical-types.schema.json) | JSON Schema for the runtime/output types the data-model schema leaves out of scope |

## Plans

Implementation plans that build the spec, under [`plans/`](plans/):

| Plan | Status | Summary |
|---|---|---|
| [2026-07-05-tmx_runtime_implementation](plans/2026-07-05-tmx_runtime_implementation/plan.md) | Draft | The full TMX runtime and CLI as a five-crate Cargo workspace — 32 task packages across 7 milestones, ordered so the reviewability spine (workspace, schema, core, the first runnable `tmx run` slice) leads. |
