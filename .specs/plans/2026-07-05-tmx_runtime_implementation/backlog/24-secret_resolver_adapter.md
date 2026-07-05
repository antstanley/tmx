# Task 24 — Secret resolver adapter (`env` / `file` / provider seam)

**Plan:** [plan.md](../plan.md) · **Certificate:** [24-secret_resolver_adapter-certificate.md](24-secret_resolver_adapter-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Secret resolution; [04-execution-engine.md](../../../04-execution-engine.md) §Secrets & masking (adapter side)
**Depends on:** 05, 17
**Produces:** the `SecretResolver` adapter — extending the minimal `env` resolver from task 17 with the `file` source and the provider trait seam — wired so resolved secrets register with the Masker end to end
**Pointers:** `crates/tmx-adapters/src/secret.rs` (new), `crates/tmx-cli/src/compose.rs` (wire into the bundle)

## Steps

- [ ] Extend the task-17 `env` `SecretResolver` with the `file` source (a path) and a provider trait seam (`aws-sm`/`vault`/… as future adapters) so the backend set stays open.
- [ ] Never accept a secret as a plain CLI flag (a process-table leak); ad-hoc injection sets a host env var referenced by a `secretSource: { env: … }`.
- [ ] Wire the adapter into the composition root so the runner resolves only the names a task lists and registers each with the Masker.
- [ ] Add tests confirming an `env`/`file` secret resolves and is masked in output, and that an unrequested secret is never resolved.

## Definition of done

- [ ] An `env` and a `file` secret resolve for a task that lists them, and the value is redacted everywhere it would be emitted end to end via the CLI.
- [ ] An unrequested secret name is never resolved or bound into a task's scope (negative space), and the provider seam compiles without a concrete backend.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run a flow whose task requests an `env` secret and echoes it, and confirm the value is masked in stdout and the run store.
