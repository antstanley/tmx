# Task 31 — Analysis, scaffold, and resource commands

**Plan:** [plan.md](../plan.md) · **Certificate:** [31-analysis_and_scaffold_commands-certificate.md](31-analysis_and_scaffold_commands-certificate.md)

**Implements:** [07-cli.md](../../../07-cli.md) §Command → use case mapping (`validate`/`inspect`/`init`/`fmt`/`list`/`context`/`secrets`/`provider`/`version`), §Configuration; [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Cross-cutting driven ports (`ManageProviders`)
**Depends on:** 13, 14, 15, 25
**Produces:** the remaining CLI surface — `validate`, `inspect`, `init`, `fmt`, `list`, `context show`, `secrets list`, `provider`, `version`/`help` — plus the layered configuration resolution
**Pointers:** `crates/tmx-cli/src/commands/` (one thin module per command, new), `crates/tmx-cli/src/config.rs` (layering)

## Steps

- [x] Implement `tmx validate` (`ValidateArtifacts`), `tmx inspect` (`InspectFlow`: resolved env+context, ordered plan, inputs, secrets-needed), and `tmx list` (`Discover`: flows/tasks/inputs/providers).
- [x] Implement `tmx init` (`ScaffoldFlow`: single-file or folder starter), `tmx fmt` (`FormatArtifact`: `SourceLoader` → re-emit, loss-free across the four formats), and `tmx provider` (`ManageProviders`: registry read/write + manifest validation).
- [x] Implement `tmx context show` / `tmx secrets list` (projections of `InspectFlow` with masked secrets) and `tmx version` / `tmx help` (CLI + supported spec version).
- [x] Implement the layered config resolution (flags > `TMX_*` env > project `tmx.config.*` > user > system) into one effective config the composition root consumes, including named profiles and registered-name → path mappings.

## Definition of done

- [x] Each command maps to its use case and produces the documented output, `tmx fmt` round-trips a Flow across all four formats without loss, and the config layers resolve highest-to-lowest.
- [x] `tmx validate` and `tmx inspect` fail-fast on a malformed artifact, `tmx secrets list` shows masked values only (negative space), and an unknown command/flag is an exit-2 usage error raised by the CLI.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run each command against a sample Flow, round-trip it through `tmx fmt` in all four formats, and confirm the config precedence with an overriding flag and env var.
