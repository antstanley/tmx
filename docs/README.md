# TMX documentation

The TMX documentation tree. Two layers:

- **The data model and design drafts** — the language-neutral definition of *what TMX is* and *how it
  should behave*: the JSON Schemas, the schema rationale, and the CLI/runtime design proposals.
- **The implementation specification** ([`specs/`](./specs)) — the formal, canonical blueprint for the
  **Rust + Tiger Style** implementation of the runtime and CLI. It commits the decisions the drafts
  deferred (host language, development style) and renders the design as concrete crates, port traits,
  types, limits, and algorithms.

> TMX is an early-stage spec with **no runtime yet**. The `specs/` set describes the *intended*
> implementation (Status: Draft on every page), consistent with the drafts it formalises.

## Implementation specification — `specs/`

The canonical Rust implementation blueprint. Read [`specs/00-overview.md`](./specs/00-overview.md)
first.

| Page | Topic |
|---|---|
| [00-overview.md](./specs/00-overview.md) | What the implementation is; the hexagon; goals/non-goals; reading order |
| [01-domain-model.md](./specs/01-domain-model.md) | Input data model + runtime entities as Rust types; IDs; Pipeline lifecycle |
| [02-crate-architecture.md](./specs/02-crate-architecture.md) | Cargo workspace, crate boundaries, the dependency rule, async model, composition root |
| [03-loading-and-preflight.md](./specs/03-loading-and-preflight.md) | SourceLoader, `kind`-dispatch, reference resolution, directory assembly, fail-fast preflight |
| [04-execution-engine.md](./specs/04-execution-engine.md) | The `PipelineRunner` algorithm, scopes & interpolation, secrets & masking, hooks, the limits table, invariants |
| [05-fan-out-and-eval.md](./specs/05-fan-out-and-eval.md) | `map` fan-out, `eval` measurement, the shared MatcherEngine, scorers, the Scheduler |
| [06-ports-and-adapters.md](./specs/06-ports-and-adapters.md) | Every driven port as a trait + its built-in adapter; the TaskDispatcher; provider execution |
| [07-cli.md](./specs/07-cli.md) | The `tmx` command surface, flags, output contract, configuration, exit codes |
| [08-errors-and-observability.md](./specs/08-errors-and-observability.md) | Error categories → exit codes, the event stream, reporters, the run store, masking |
| [architecture-principles.md](./specs/architecture-principles.md) | Hexagonal layering + Tiger Style tenets + Rust conventions (cross-cutting) |
| [development-guidelines.md](./specs/development-guidelines.md) | Tiger Style for Rust: toolchain, code style, limits, version control, testing, definition of done |
| [canonical-types.schema.json](./specs/canonical-types.schema.json) | JSON Schema for the **runtime/output** types the data-model schema leaves out of scope |

## Data model and design drafts

| Path | What |
|---|---|
| [tmx.schema.json](./tmx.schema.json) | Core data model — Flow / Task / Context / Environment (JSON Schema Draft 2020-12), spec 0.2.0 |
| [tmx-provider.schema.json](./tmx-provider.schema.json) | Environment provider manifest schema |
| [SCHEMA.md](./SCHEMA.md) | Data-model design decisions, interpretations, and open questions |
| [CLI.md](./CLI.md) | Proposed `tmx` command-line interface (design draft) — formalised in [`specs/07-cli.md`](./specs/07-cli.md) |
| [RUNTIME.md](./RUNTIME.md) | Proposed execution engine, hexagonal ports & adapters (design draft) — formalised across [`specs/`](./specs) |
| [comparison.md](./comparison.md) | Task/workflow-runner landscape and TMX positioning |
| [examples/](./examples) | Validated worked examples in all four source formats — see its [README](./examples/README.md) |

## Relationship between the layers

The drafts ([`CLI.md`](./CLI.md), [`RUNTIME.md`](./RUNTIME.md), [`SCHEMA.md`](./SCHEMA.md)) are the
**design rationale** — the "why", kept language-neutral. The [`specs/`](./specs) set is the
**implementation blueprint** — the Rust "what", which links back to the drafts rather than restating
them. Where they overlap, a spec page states how the Rust implementation realises the draft's design
and cites the relevant draft decision.

## Validating the schemas and examples

```bash
scripts/validate.sh                     # meta-schema + all examples + cross-format parity
scripts/validate.sh path/to/file.yaml   # validate a single file (dispatched by `kind`)
```

See the root [README](../README.md#version-control--pre-push) for the jj/git pre-push gate.
