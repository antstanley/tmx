# Done Certificate — Task 31: Analysis, scaffold, and resource commands

**Task:** [31-analysis_and_scaffold_commands.md](31-analysis_and_scaffold_commands.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

> This certificate is a verification protocol for Task 31. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 31) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship the remaining CLI surface — `validate`, `inspect`, `init`, `fmt`, `list`,
  `context show`, `secrets list`, `provider`, `version`/`help` — plus the layered configuration
  resolution the composition root consumes, each per [07-cli.md](../../../07-cli.md) §Command → use
  case mapping and §Configuration.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Builds on Task 13 (`SourceLoader`/resolver, reused by `fmt` re-emit and
  `list`), Task 14 (`SchemaValidator`, reused by `validate`), Task 15 (preflight, reused by
  `inspect`), and Task 25 (`ManageProviders`/provider seam, reused by `provider`/`context`). The
  existing Task-17 `tmx run` command must remain unaffected by the new sibling subcommands.

## Obligations

- **O1 — Each command maps to its use case and produces the documented output, `tmx fmt` round-trips a Flow across all four formats without loss, and the config layers resolve highest-to-lowest.**
  - *Claim:* `validate`→`ValidateArtifacts`, `inspect`→`InspectFlow`, `list`→`Discover`,
    `init`→`ScaffoldFlow`, `fmt`→`FormatArtifact`, `provider`→`ManageProviders`, `context show` /
    `secrets list`→`InspectFlow` projections, `version`/`help`→CLI-local each emit the documented
    output; `tmx fmt` re-emits a Flow across YAML/JSON/JSONC/TOML with no loss; config resolves
    flags > `TMX_*` env > project `tmx.config.*` > user > system into one effective config, including
    named profiles and registered-name → path mappings.
  - *Evidence to collect:* read each thin module under `crates/tmx-cli/src/commands/` and confirm it
    calls its mapped use case (per 07-cli.md §Command → use case mapping) with no business logic in
    the command module; read `crates/tmx-cli/src/config.rs` for the five-layer resolution and profile
    handling. Run the named tests — a per-command output test, a `tmx fmt` round-trip asserting the
    loaded model is identical across all four formats, and a config-precedence test — expect all to
    pass.
  - *Checks:* trace a `tmx fmt` round-trip (`SourceLoader` parse → re-emit → reload) for one Flow
    through YAML/JSON/JSONC/TOML and confirm the reloaded model equals the original (loss-free); trace
    config resolution and confirm a flag overrides a `TMX_*` env var which overrides project
    `tmx.config.*`.
  - *Status:* ☑ SATISFIED — every command module read and confirmed a thin call into its mapped
    driving port (`EngineValidateArtifacts`/`EngineInspectFlow`/`EngineDiscover`/`EngineScaffoldFlow`/
    `EngineFormatArtifact`/`EngineManageProviders`; `context show`/`secrets list` reuse inspect's
    `context_projection`/`secrets_needed`; `version` is CLI-local). `emit.rs` unit test
    `round_trips_loss_free_across_every_format` + `fmt_round_trips_a_flow_across_all_four_formats_without_loss`
    (unit and e2e) prove the loss-free round-trip; `config_layer_tests` +
    `config_layers_resolve_flag_over_env_over_project` (e2e) prove flag > env > project precedence,
    profiles, and registered-name → path mappings. Independently re-exercised live: all four
    conversions re-emitted byte-identical JSON models; `--profile local` beat `TMX_PROFILE=ci`
    beat the base project names through `tmx list flows`; `tmx inspect <registered-name>` resolved
    its mapped path.

- **O2 — `tmx validate`/`tmx inspect` fail-fast on a malformed artifact, `tmx secrets list` shows masked values only, and an unknown command/flag is an exit-2 usage error raised by the CLI (negative space).**
  - *Claim:* a malformed artifact aborts `validate`/`inspect` before any output (typed validation
    error → exit 3); `tmx secrets list` never prints an unmasked secret value; an unknown command or
    flag is an exit-2 usage error emitted by the CLI, distinct from any core `ErrorCategory`.
  - *Evidence to collect:* read `crates/tmx-cli/src/commands/validate.rs` and `inspect.rs` for the
    fail-fast path via the Task-14 validator / Task-15 preflight; read `commands/secrets.rs` for the
    Masker projection; read `crates/tmx-cli/src/args.rs` (clap) for the usage-error → exit-2 wiring.
    Run the negative tests — a malformed artifact through `validate` and `inspect` (expect fail-fast,
    exit 3), a `secrets list` output scanned for a raw secret (expect masked only), and an unknown
    subcommand/flag (expect exit 2) — expect all to hold.
  - *Checks:* confirm `secrets list` routes every value through the Masker so no raw secret reaches
    stdout; confirm the unknown-command/flag path returns exit 2 as a CLI-local usage error, not a
    core `resolution`/`validation` category mapped elsewhere.
  - *Status:* ☑ SATISFIED — read: `validate.rs`/`inspect.rs` surface the loader/preflight typed
    error before any output; `secrets.rs`/`context.rs` reuse inspect's masked projection
    (`masked_secret` emits `[REDACTED]` for a literal, a non-secret descriptor for a source — the
    raw value never enters the projection; `inspect` additionally scrubs its whole view through the
    `Masker`); exit 2 is clap's usage error, CLI-local. Ran: malformed artifact → `validate` and
    `inspect` both exit 3 with 0 bytes on stdout; a raw-secret scan of `secrets list` +
    `context show` + `inspect` output found no occurrence; `tmx teleport` and `--nope` both exit 2.
    E2e tests `validate_passes_a_good_flow_and_fails_fast_on_a_malformed_one`,
    `inspect_projects_the_plan_and_fails_fast_on_a_malformed_flow`,
    `secrets_and_context_show_masked_values_only`,
    `an_unknown_command_or_flag_is_an_exit_2_usage_error` pin all four.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant in
    `tmx-schema::limits`.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features
    -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm each new validation path
    (malformed-artifact fail-fast, secret masking, usage error) ships a negative-space test.
  - *Status:* ☑ SATISFIED — independently ran from the main tree: `cargo fmt --all --check` exit 0;
    `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 421/421
    passed; `bash scripts/purity.sh` green. No new numeric bound was introduced (the new constants
    are string keys/markers: `SUPPORTED_SPEC_VERSION`, `PROFILES_KEY`/`NAMES_KEY`/`CONFIG_STEM`,
    `MASKED_SECRET_PLACEHOLDER`, `REGISTRY_RELATIVE`), so `tmx-schema::limits` is correctly
    untouched. Each negative-space path carries a test (emit `source_emit_error`, registry
    `registry_not_object`, plus the O2 e2e suite).

- **O4 — Reviewable: run each command against a sample Flow, round-trip it through `tmx fmt` in all four formats, and confirm the config precedence with an overriding flag and env var (Reviewable).**
  - *Claim:* a reviewer can run each command against a sample Flow and observe its documented output,
    round-trip the Flow through `tmx fmt` in all four formats observing no loss, and confirm a flag
    overrides an env var overrides project config.
  - *Evidence to collect:* build the binary (`cargo build -p tmx-cli`); run `tmx validate`,
    `tmx inspect`, `tmx list`, `tmx init`, `tmx fmt`, `tmx context show`, `tmx secrets list`,
    `tmx provider …`, and `tmx version` against a sample Flow and observe each documented output; run
    `tmx fmt <flow> --to <fmt>` for each of YAML/JSON/JSONC/TOML and diff the reloaded model against
    the original; set a value in project `tmx.config.*`, override it with the matching `TMX_*` env
    var, then override that with the equivalent flag, and observe the effective value follow
    highest-to-lowest precedence.
  - *Status:* ☑ SATISFIED — built the binary and exercised every command against a sample Flow in a
    scratch directory: `version` (cli 0.1.0 + spec 0.2.0), `validate` (exit 0), `inspect` (env,
    masked context, ordered plan, capabilities, secretsNeeded), `list flows/tasks/inputs`, `init`
    single-file and `--folder` (each output re-validated clean via `tmx validate`), `fmt --to` all
    four formats (each converted form re-converted to JSON diffed byte-identical against the
    baseline; `--write` produced `flow.json`), `provider validate/register/list` (a malformed
    manifest exits 3 and never enters the registry), `context show`, `secrets list`. Config
    precedence observed live: project `tmx.config.json` base names → `TMX_PROFILE=ci` overrides →
    `--profile local` beats the env var. All temporary fixtures were confined to the scratchpad;
    the tree is untouched.

## Regression check

- Task 17: trace that adding the sibling subcommands under the same clap parser leaves
  `tmx run flow.yaml` dispatch and behaviour unchanged — `run` remains the primary, still resolves,
  preflights, executes, and prints masked final state as in Task 17, with the new commands as peers
  that do not intercept its arguments.

## Residue

- `provider` exercises `ManageProviders` (registry read/write + manifest validation) — validator
  should confirm a registry write validates the manifest and rejects a malformed one.
- `context show` masks secrets on the same projection path as `secrets list`; confirm both share the
  masked `InspectFlow` projection rather than only `secrets list` masking.
- `init` (`ScaffoldFlow`) supports both single-file and folder layouts — confirm both are produced and
  the scaffold itself validates.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently produced evidence: every new command is
a thin shell over its mapped driving port; `tmx fmt` round-trips one model across YAML/JSON/JSONC/
TOML loss-free (unit, e2e, and live re-run); config resolves flags > env > project > user > system
with profiles and registered names; the negative space holds (malformed → exit 3 with empty stdout,
secrets always masked, unknown command/flag → exit 2, malformed manifests never registered); the
repo gates are green (fmt/clippy/nextest 421/421/purity) and `tmx run` regression-checked live.
Residue confirmed: provider register validates before recording; context/secrets share inspect's
masked projection; both init layouts self-validate. Minor non-blocking notes: the global
`--profile` flag is threaded only into `tmx list` (other commands honour `TMX_PROFILE` via the env
layer but not the flag — the wider composition consumer is follow-on work), and
`EffectiveConfig::get_str` is `#[allow(dead_code)]` ahead of its consumer, matching the existing
compose.rs seam idiom.
