# Task 25 — Environment providers and the ephemeral lifecycle

**Plan:** [plan.md](../plan.md) · **Certificate:** [25-environment_providers-certificate.md](25-environment_providers-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Environment and provider execution; [07-cli.md](../../../07-cli.md) §Command → use case mapping (`tmx env`); [02-crate-architecture.md](../../../02-crate-architecture.md) §tmx-schema (provider manifest types)
**Depends on:** 12, 15, 16, 17
**Produces:** the `EnvironmentProvider` port with `BinaryProvider` and `FlowProvider` adapters, the provider manifest types, and the ephemeral deploy/clean/destroy lifecycle wrapping `RunFlow`, driven by `tmx env`
**Pointers:** `crates/tmx-schema/src/provider.rs` (new, manifest types), `crates/tmx-adapters/src/provider/` (new, Binary/Flow), `crates/tmx-cli/src/commands/env.rs` (new)

## Steps

- [ ] Define the provider manifest types (mirroring `tmx-provider.schema.json`) in `tmx-schema`, and validate `environment.options` against the manifest's `optionsSchema` at preflight.
- [ ] Implement `BinaryProvider` (invoke the manifest binary with the method subcommand, passing the resolved environment/options; the process result is the method result) and `FlowProvider` (run the method's inline tasks / referenced Flow through the same `PipelineRunner`, inheriting the recursion depth bound).
- [ ] Wrap the pipeline with the ephemeral lifecycle — `deploy → run → clean` by default, `--keep`, `--no-deploy`, `--local` — running `clean`/`destroy` best-effort even after a failed or cancelled run, with the context `destroy` hook still firing.
- [ ] Implement `tmx env` mapping the provider methods 1:1 plus `up`/`down` aggregates, returning `EnvironmentError` (exit 5) for a failed method, distinct from a pipeline `RunFailure`.

## Definition of done

- [ ] `tmx env` drives a `BinaryProvider` method and a `FlowProvider` method (a method body running as a Flow), and a `tmx run` provisions and cleans a standing environment per `--keep`/`--no-deploy`/`--local`.
- [ ] A failed provider method is `EnvironmentError` (exit 5) not a `RunFailure`, `clean`/`destroy` still run after a failed run, and an `options` block violating the `optionsSchema` is rejected at preflight (negative space).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run `tmx env deploy`/`clean` against a binary provider and a flow provider, and confirm the exit code and best-effort teardown on a forced failure.
