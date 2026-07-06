//! Argument parsing — the `clap` surface for the `tmx` binary (07 §Implementation layout).
//!
//! The CLI is hybrid (07 §Command → use case mapping): flat primaries for high-frequency actions and
//! noun groups for resource areas. Task 17 lands the load-bearing primary, [`Command::Run`]; the rest
//! of the surface arrives with its own tasks. This module owns only the *shape* of the arguments —
//! `main` maps a parsed command to a use case and its result to an exit code.

use clap::{Parser, Subcommand, ValueEnum};

/// The `tmx` command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "tmx",
    about = "Run TMX Flows: one runtime for exec/assert/fetch/file/store/chat pipelines.",
    version
)]
pub struct Cli {
    /// Select a named configuration profile (07 §Configuration). Overrides `TMX_PROFILE` and a
    /// `profile` key in the project `tmx.config.*`. Global: it applies to every subcommand.
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,

    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The top-level commands. Task 17 implements `run`; task 25 adds `env` (provider lifecycle).
// `Run` carries the full `tmx run` flag surface, so it is far larger than the other variants — but a
// parsed `Cli` is a single short-lived value the process constructs once at startup, never stored in
// bulk, so the size asymmetry costs nothing. Boxing it would fight `clap`'s derive for a subcommand's
// `Args` field; the allow is the idiomatic choice for a one-shot arg tree.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a Flow end to end: load, preflight, execute, and print the masked final state to stdout.
    Run(RunArgs),
    /// Statically lint a Flow: resolution + dataflow analysis beyond schema (07 §`tmx lint`).
    Lint(LintArgs),
    /// Drive a Flow's environment provider through its lifecycle methods (07 §`tmx env`).
    Env(EnvArgs),
    /// Query the local run store: list, show, dump state/logs, prune, or remove runs (07 §Pipeline runs).
    Runs(RunsArgs),
    /// Validate one or more artifacts against the schema (`ValidateArtifacts`; 07 §`tmx validate`).
    Validate(ValidateArgs),
    /// Inspect a Flow's resolved plan: env+context, ordered tasks, inputs, secrets-needed (07 §`tmx inspect`).
    Inspect(InspectArgs),
    /// List Flows / tasks / inputs / providers (`Discover`; 07 §`tmx list`).
    List(ListArgs),
    /// Scaffold a starter Flow, single-file or folder layout (`ScaffoldFlow`; 07 §`tmx init`).
    Init(InitArgs),
    /// Reformat an artifact, converting losslessly across the four formats (`FormatArtifact`; 07 §`tmx fmt`).
    Fmt(FmtArgs),
    /// Manage the provider registry: list/show/register/remove/validate (`ManageProviders`; 07 §`tmx provider`).
    Provider(ProviderArgs),
    /// Show a Flow's resolved context (env + masked secrets) — an `InspectFlow` projection.
    Context(ContextArgs),
    /// List a Flow's needed secrets, masked (an `InspectFlow` projection; 07 §Command mapping).
    Secrets(SecretsArgs),
    /// Print the CLI version and the supported TMX spec version (CLI-local; 07 §Command mapping).
    Version,
}

/// A [`SourceKind`](tmx_core::ports::driven::SourceKind) selector on the CLI surface — the `--to`
/// target format of `tmx fmt`. Kept local so `clap`'s derive owns the token vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SourceKindArg {
    /// YAML.
    Yaml,
    /// JSON.
    Json,
    /// JSON with comments.
    Jsonc,
    /// TOML.
    Toml,
}

impl SourceKindArg {
    /// Map the CLI arg to the port [`SourceKind`](tmx_core::ports::driven::SourceKind).
    #[must_use]
    pub fn to_kind(self) -> tmx_core::ports::driven::SourceKind {
        use tmx_core::ports::driven::SourceKind;
        match self {
            SourceKindArg::Yaml => SourceKind::Yaml,
            SourceKindArg::Json => SourceKind::Json,
            SourceKindArg::Jsonc => SourceKind::Jsonc,
            SourceKindArg::Toml => SourceKind::Toml,
        }
    }
}

/// What `tmx list` enumerates — the CLI mirror of
/// [`DiscoverKind`](tmx_core::ports::driving::DiscoverKind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum DiscoverKindArg {
    /// The Flows discoverable in the working directory (the default).
    #[default]
    Flows,
    /// A Flow's tasks.
    Tasks,
    /// A Flow's declared inputs.
    Inputs,
    /// The registered providers.
    Providers,
}

impl DiscoverKindArg {
    /// Map the CLI arg to the [`DiscoverKind`](tmx_core::ports::driving::DiscoverKind) use-case input.
    #[must_use]
    pub fn to_kind(self) -> tmx_core::ports::driving::DiscoverKind {
        use tmx_core::ports::driving::DiscoverKind;
        match self {
            DiscoverKindArg::Flows => DiscoverKind::Flows,
            DiscoverKindArg::Tasks => DiscoverKind::Tasks,
            DiscoverKindArg::Inputs => DiscoverKind::Inputs,
            DiscoverKindArg::Providers => DiscoverKind::Providers,
        }
    }
}

/// Arguments for `tmx validate` — the artifacts to check against the schema (07 §`tmx validate`).
#[derive(Debug, Default, Parser)]
pub struct ValidateArgs {
    /// The artifact files to validate. When omitted, the Flow is resolved by the same search order
    /// as `tmx run`, and that single Flow is validated.
    pub paths: Vec<String>,
}

/// Arguments for `tmx inspect` — the Flow whose resolved plan is projected (07 §`tmx inspect`).
#[derive(Debug, Default, Parser)]
pub struct InspectArgs {
    /// The Flow to inspect; resolved by the same search order as `tmx run` when omitted.
    pub flow: Option<String>,
    /// Explicit Flow file, taking precedence over the positional argument and `$TMX_FLOW`.
    #[arg(short = 'f', long = "file")]
    pub file: Option<String>,
}

/// Arguments for `tmx list` — the discovery kind and an optional scoping Flow (07 §`tmx list`).
#[derive(Debug, Default, Parser)]
pub struct ListArgs {
    /// What to list: `flows` (the default), `tasks`, `inputs`, or `providers`.
    #[arg(value_enum, default_value_t = DiscoverKindArg::Flows)]
    pub kind: DiscoverKindArg,
    /// The Flow to scope `tasks`/`inputs` to; resolved by the `tmx run` search order when omitted.
    pub flow: Option<String>,
    /// Explicit Flow file, taking precedence over the positional argument and `$TMX_FLOW`.
    #[arg(short = 'f', long = "file")]
    pub file: Option<String>,
}

/// Arguments for `tmx init` — scaffold a starter Flow (07 §`tmx init`).
#[derive(Debug, Default, Parser)]
pub struct InitArgs {
    /// The starter Flow's name (the file stem for a single-file scaffold; the directory name for a
    /// folder layout). Defaults to `flow`.
    pub name: Option<String>,
    /// Scaffold a folder layout (separate `environment.*`/`context.*`/task files) instead of a
    /// single-file Flow.
    #[arg(long)]
    pub folder: bool,
    /// The directory to scaffold into (defaults to the current working directory).
    #[arg(long = "dir", value_name = "DIR")]
    pub dir: Option<String>,
}

/// Arguments for `tmx fmt` — reformat / convert an artifact (07 §`tmx fmt`).
#[derive(Debug, Default, Parser)]
pub struct FmtArgs {
    /// The artifact file to format. When omitted, the Flow is resolved by the `tmx run` search order.
    pub path: Option<String>,
    /// Convert to this target format (loss-free across the four formats). When omitted, the artifact
    /// is reformatted in its own format.
    #[arg(long = "to", value_enum)]
    pub to: Option<SourceKindArg>,
    /// Write the formatted output back to the source file (converting its extension on a `--to`)
    /// instead of printing it to stdout.
    #[arg(long)]
    pub write: bool,
}

/// Arguments for `tmx provider` — a provider-registry operation (07 §`tmx provider`).
#[derive(Debug, Parser)]
pub struct ProviderArgs {
    /// The registry operation to perform.
    #[command(subcommand)]
    pub command: ProviderCommand,
}

/// The `tmx provider` sub-actions, one per
/// [`ProviderOp`](tmx_core::ports::driving::ProviderOp) variant.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List registered providers (name → manifest path).
    List,
    /// Show one registered provider's manifest by name.
    Show {
        /// The registered provider name.
        name: String,
    },
    /// Register a provider manifest at a path (validated before it is recorded).
    Register {
        /// The manifest file path.
        path: String,
    },
    /// Remove a registered provider by name.
    Remove {
        /// The registered provider name.
        name: String,
    },
    /// Validate a provider manifest at a path without registering it.
    Validate {
        /// The manifest file path.
        path: String,
    },
}

/// Arguments for `tmx context` — the context projection group (07 §Command mapping).
#[derive(Debug, Parser)]
pub struct ContextArgs {
    /// The context sub-action to perform.
    #[command(subcommand)]
    pub command: ContextCommand,
}

/// The `tmx context` sub-actions.
#[derive(Debug, Subcommand)]
pub enum ContextCommand {
    /// Show the Flow's resolved context: env vars and masked secrets.
    Show {
        /// The Flow whose context is projected; resolved by the `tmx run` search order when omitted.
        flow: Option<String>,
        /// Explicit Flow file, taking precedence over the positional argument and `$TMX_FLOW`.
        #[arg(short = 'f', long = "file")]
        file: Option<String>,
    },
}

/// Arguments for `tmx secrets` — the secrets projection group (07 §Command mapping).
#[derive(Debug, Parser)]
pub struct SecretsArgs {
    /// The secrets sub-action to perform.
    #[command(subcommand)]
    pub command: SecretsCommand,
}

/// The `tmx secrets` sub-actions.
#[derive(Debug, Subcommand)]
pub enum SecretsCommand {
    /// List the Flow's needed secrets, masked (never a raw value).
    List {
        /// The Flow whose secrets are projected; resolved by the `tmx run` search order when omitted.
        flow: Option<String>,
        /// Explicit Flow file, taking precedence over the positional argument and `$TMX_FLOW`.
        #[arg(short = 'f', long = "file")]
        file: Option<String>,
    },
}

/// The runtime `produces`-conformance mode selected by `--check-produces[=warn|strict]` (04 §`produces`
/// conformance) — the CLI mirror of [`tmx_core::ProducesCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CheckProducesArg {
    /// A `produces` mismatch is a non-blocking warning; the run continues (the bare-flag default).
    Warn,
    /// A `produces` mismatch fails the task.
    Strict,
}

impl CheckProducesArg {
    /// Map the CLI arg to the engine [`ProducesCheck`](tmx_core::ProducesCheck) the runner honours.
    #[must_use]
    pub fn to_check(self) -> tmx_core::ProducesCheck {
        match self {
            CheckProducesArg::Warn => tmx_core::ProducesCheck::Warn,
            CheckProducesArg::Strict => tmx_core::ProducesCheck::Strict,
        }
    }
}

/// Arguments for `tmx lint` — the deeper static pass (07 §`tmx lint`; 03 §`lint`).
#[derive(Debug, Default, Parser)]
pub struct LintArgs {
    /// The Flow to lint: a file, or resolved by the same search order as `tmx run` when omitted.
    pub flow: Option<String>,

    /// Explicit Flow file, taking precedence over the positional argument and `$TMX_FLOW`.
    #[arg(short = 'f', long = "file")]
    pub file: Option<String>,

    /// Promote every lint warning to an error, so any finding exits 3 (03 §`lint`).
    #[arg(long)]
    pub strict: bool,
}

/// Arguments for `tmx run` — the core surface plus the ephemeral-environment lifecycle flags
/// (07 §`tmx run`; 06 §Ephemeral lifecycle). The full run-flag depth (inputs, env, slicing, dry-run,
/// matrix, concurrency, …) is task 30.
#[derive(Debug, Default, Parser)]
pub struct RunArgs {
    /// The Flow to run: a file (`flow.yaml`/`.yml`/`.json`/`.jsonc`/`.toml`) or a directory layout.
    /// When omitted, the flow is resolved by the search order (see the resolver).
    pub flow: Option<String>,

    /// Explicit Flow file/directory, taking precedence over the positional argument and `$TMX_FLOW`.
    #[arg(short = 'f', long = "file")]
    pub file: Option<String>,

    /// Keep the ephemeral environment up after the run: `deploy` runs, but `clean` is skipped.
    #[arg(long)]
    pub keep: bool,

    /// Reuse a standing environment: skip both `deploy` and `clean` (the provider is not provisioned).
    #[arg(long = "no-deploy")]
    pub no_deploy: bool,

    /// Run in the current process with no provider lifecycle at all (`--no-env` is an alias).
    #[arg(long, visible_alias = "no-env")]
    pub local: bool,

    /// The stdout reporter: `pretty` (human summary; TTY default), `json` (final state object; pipe
    /// default), or `ndjson` (one event per line). Overrides `TMX_FORMAT` and the TTY default.
    #[arg(long, value_enum)]
    pub format: Option<FormatArg>,

    /// Force ANSI colour on the stderr progress, overriding the TTY / `NO_COLOR` default.
    #[arg(long, conflicts_with = "no_color")]
    pub color: bool,

    /// Disable ANSI colour on the stderr progress (as `NO_COLOR` does).
    #[arg(long = "no-color")]
    pub no_color: bool,

    /// Do not record this run in the local run store (`./.tmx/runs/`): no snapshot, no event log.
    #[arg(long = "no-store")]
    pub no_store: bool,

    /// Check each task's output against its `produces` schema at run time (04 §`produces` conformance):
    /// a bare `--check-produces` warns on a mismatch, `--check-produces=strict` fails the task, and an
    /// absent flag checks nothing.
    #[arg(long = "check-produces", value_enum, num_args = 0..=1, default_missing_value = "warn")]
    pub check_produces: Option<CheckProducesArg>,

    /// Cancel the run if it exceeds this wall-clock budget, as a duration (`500ms`/`30s`/`5m`/`1h`, or
    /// bare seconds). On expiry the run stops dispatching new work, in-flight adapters get the grace
    /// window, `destroy` fires, and the run exits 124 (06 §Concurrency, cancellation, timeouts).
    #[arg(long)]
    pub timeout: Option<String>,

    /// Grace window between a cancel signal (`--timeout` or SIGINT) and the hard stop, as a duration.
    /// Overrides the `CANCEL_GRACE_MS` default (5s); `--grace 0` forces an immediate hard stop.
    #[arg(long)]
    pub grace: Option<String>,

    /// Supply a declared input, coerced to its declared `type`: `--input k=v` (a string, coerced) or
    /// `--input k:=<json>` (a raw JSON value). Repeatable; a later value wins on a repeated key.
    #[arg(long = "input", short = 'i', value_name = "K=V")]
    pub input: Vec<String>,

    /// Read declared inputs from a JSON object file; each key is an input, coerced to its declared
    /// `type`. Individual `--input` flags override a value the file supplies.
    #[arg(long = "inputs-file", value_name = "FILE")]
    pub inputs_file: Option<String>,

    /// Override a context `env` var for this run: `--env K=V`. Repeatable.
    #[arg(long = "env", value_name = "K=V")]
    pub env: Vec<String>,

    /// Seed the Pipeline state from a JSON object file (`--state-in`), re-validated on read; a later
    /// task then reads a prior task's state via `${{ tasks.NAME.field }}`.
    #[arg(long = "state-in", value_name = "FILE")]
    pub state_in: Option<String>,

    /// Dump the masked final Pipeline state to a JSON file at run end (`--state-out`).
    #[arg(long = "state-out", value_name = "FILE")]
    pub state_out: Option<String>,

    /// Run only these tasks (by name); repeatable. Slicing pairs with `--state-in`.
    #[arg(long = "only", value_name = "TASK")]
    pub only: Vec<String>,

    /// Skip these tasks (by name); repeatable.
    #[arg(long = "skip", value_name = "TASK")]
    pub skip: Vec<String>,

    /// Start the run at this task, dropping every earlier one (inclusive).
    #[arg(long = "from", value_name = "TASK")]
    pub from: Option<String>,

    /// End the run at this task, dropping every later one (inclusive).
    #[arg(long = "until", value_name = "TASK")]
    pub until: Option<String>,

    /// Resolve, validate, and print the plan; execute nothing (`--dry-run` / `-n`).
    #[arg(long = "dry-run", short = 'n')]
    pub dry_run: bool,

    /// Desugar to a bounded `map` cross-product: `--matrix key=v1,v2`. Repeatable axes form the
    /// cross-product, each combination binding `${{ matrix.<key> }}`. An authored `map` wins (a
    /// stderr warning; `--matrix` is ignored).
    #[arg(long = "matrix", value_name = "K=V1,V2")]
    pub matrix: Vec<String>,

    /// Global concurrency cap for `map`/`eval` fan-out (`--concurrency N`), clamped to `CONCURRENCY_MAX`.
    #[arg(long)]
    pub concurrency: Option<u32>,

    /// Force `continueOnError` across every task: a task failure is recorded and the run continues.
    #[arg(long = "continue-on-error")]
    pub continue_on_error: bool,

    /// Narrow the state-size cap for this run (`--max-state-size <bytes>`), clamped to the hard
    /// `STATE_SIZE_MAX_BYTES` ceiling. A plain byte count.
    #[arg(long = "max-state-size", value_name = "BYTES")]
    pub max_state_size: Option<u64>,

    /// Re-run the Flow whenever its source file changes (`--watch`); each re-run is a full run with its
    /// own record. SIGINT stops the watcher; the process exits with the most recent run's code.
    #[arg(long)]
    pub watch: bool,
}

/// Arguments for `tmx runs` — a query against the local run store (07 §Pipeline runs; 08 §Run store).
#[derive(Debug, Parser)]
pub struct RunsArgs {
    /// The run-store query to perform.
    #[command(subcommand)]
    pub command: RunsCommand,
}

/// The `tmx runs` sub-actions, one per [`RunQuery`](tmx_core::ports::driving::RunQuery) variant.
#[derive(Debug, Subcommand)]
pub enum RunsCommand {
    /// List stored runs, chronological by id.
    List,
    /// Show one run's full record (the masked final-state snapshot plus its metadata).
    Show {
        /// The run id (a UUIDv7).
        id: String,
    },
    /// Dump one run's masked final state.
    State {
        /// The run id (a UUIDv7).
        id: String,
    },
    /// Replay one run's masked event log.
    Logs {
        /// The run id (a UUIDv7).
        id: String,
    },
    /// Prune runs older than the retention window.
    Prune,
    /// Remove one run by id.
    Rm {
        /// The run id (a UUIDv7).
        id: String,
    },
}

/// The `--format` value on the CLI surface — the clap mirror of
/// [`tmx_adapters::sink::Format`](crate::compose). Kept local to the arg surface so `clap`'s derive
/// owns the token vocabulary; `config` maps it to the adapter [`Format`](tmx_adapters::sink::Format).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    /// Human run summary; stdout carries nothing (the human reads the stderr progress).
    Pretty,
    /// The final Pipeline state as one masked JSON object on stdout.
    Json,
    /// One masked event per line on stdout.
    Ndjson,
}

impl FormatArg {
    /// Map the CLI arg to the adapter [`Format`](tmx_adapters::sink::Format) the reporter selects on.
    #[must_use]
    pub fn to_format(self) -> tmx_adapters::sink::Format {
        use tmx_adapters::sink::Format;
        match self {
            FormatArg::Pretty => Format::Pretty,
            FormatArg::Json => Format::Json,
            FormatArg::Ndjson => Format::Ndjson,
        }
    }
}

/// Arguments for `tmx env` — a provider lifecycle method (or an `up`/`down` aggregate) against the
/// environment of a Flow (07 §`tmx env`).
#[derive(Debug, Parser)]
pub struct EnvArgs {
    /// The lifecycle method to run: a single provider method, or the `up`/`down` aggregate.
    #[arg(value_enum)]
    pub method: EnvMethod,

    /// The Flow whose `environment.provider` is driven. Resolved by the same search order as `tmx run`.
    pub flow: Option<String>,

    /// Explicit Flow file/directory, taking precedence over the positional argument and `$TMX_FLOW`.
    #[arg(short = 'f', long = "file")]
    pub file: Option<String>,
}

/// A `tmx env` method selector: the four provider lifecycle methods 1:1, plus the `up`/`down`
/// aggregates (07 §`tmx env`: "provider methods 1:1 + `up`/`down` aggregates").
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EnvMethod {
    /// Bootstrap the provider substrate.
    Bootstrap,
    /// Bring an ephemeral environment up.
    Deploy,
    /// Tear a deployed ephemeral environment down (best-effort).
    Clean,
    /// Destroy the provider substrate (best-effort).
    Destroy,
    /// Aggregate: `bootstrap` then `deploy`.
    Up,
    /// Aggregate: `clean` then `destroy` (best-effort teardown).
    Down,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_surface_is_internally_valid() {
        // clap's own debug_assert catches an ill-formed derive surface (duplicate flags, bad arg
        // specs) — a cheap compile-adjacent guard that the command tree is well-formed.
        Cli::command().debug_assert();
    }

    /// Extract the [`RunArgs`] from a parsed CLI, panicking if it was not a `run` command.
    fn run_args(cli: Cli) -> RunArgs {
        match cli.command {
            Command::Run(args) => args,
            other => panic!("expected a run command, got {other:?}"),
        }
    }

    #[test]
    fn run_parses_positional_and_file_flag() {
        // The positional flow and the -f/--file override both parse into RunArgs.
        let args = run_args(
            Cli::try_parse_from(["tmx", "run", "pipeline.yaml"]).expect("positional parses"),
        );
        assert_eq!(
            args.flow.as_deref(),
            Some("pipeline.yaml"),
            "positional flow"
        );
        assert!(args.file.is_none(), "no --file given");
        assert!(
            !args.keep && !args.no_deploy && !args.local,
            "lifecycle flags default off"
        );

        let args = run_args(
            Cli::try_parse_from(["tmx", "run", "--file", "explicit.toml"])
                .expect("the --file flag parses"),
        );
        assert_eq!(
            args.file.as_deref(),
            Some("explicit.toml"),
            "--file captured"
        );

        // Negative space: an unknown subcommand is a usage error clap rejects (CLI exit 2, not a core
        // category), never a silent no-op.
        assert!(
            Cli::try_parse_from(["tmx", "teleport"]).is_err(),
            "an unknown command is rejected"
        );
    }

    #[test]
    fn run_parses_the_ephemeral_lifecycle_flags() {
        // --keep, --no-deploy, and --local (aliased --no-env) all parse into their flags.
        let keep = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--keep"]).expect("--keep parses"),
        );
        assert!(
            keep.keep && !keep.no_deploy && !keep.local,
            "--keep sets only keep"
        );

        let no_deploy = run_args(
            Cli::try_parse_from(["tmx", "run", "--no-deploy", "flow.yaml"])
                .expect("--no-deploy parses"),
        );
        assert!(no_deploy.no_deploy, "--no-deploy is captured");

        let local = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--local"]).expect("--local parses"),
        );
        assert!(local.local, "--local is captured");
        let no_env = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--no-env"]).expect("--no-env parses"),
        );
        assert!(no_env.local, "--no-env is an alias for --local");
    }

    #[test]
    fn run_parses_the_format_and_color_flags() {
        // --format selects the reporter, mapping to the adapter Format; --color/--no-color set the
        // colour flags and are mutually exclusive.
        let ndjson = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--format", "ndjson"])
                .expect("--format ndjson parses"),
        );
        assert_eq!(
            ndjson.format.map(FormatArg::to_format),
            Some(tmx_adapters::sink::Format::Ndjson),
            "--format ndjson maps to the adapter Format"
        );
        assert!(
            !ndjson.color && !ndjson.no_color,
            "colour flags default off"
        );

        let colored = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--color"]).expect("--color parses"),
        );
        assert!(colored.color, "--color is captured");

        let no_color = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--no-color"])
                .expect("--no-color parses"),
        );
        assert!(no_color.no_color, "--no-color is captured");

        // Negative space: --color and --no-color conflict (clap rejects), and an unknown format is a
        // usage error — neither is silently accepted.
        assert!(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--color", "--no-color"]).is_err(),
            "--color and --no-color are mutually exclusive"
        );
        assert!(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--format", "yaml"]).is_err(),
            "an unknown --format value is rejected"
        );
    }

    #[test]
    fn run_parses_the_no_store_flag() {
        // `--no-store` opts the run out of recording; it defaults off.
        let default =
            run_args(Cli::try_parse_from(["tmx", "run", "flow.yaml"]).expect("bare run parses"));
        assert!(!default.no_store, "recording is on by default");
        let opted_out = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--no-store"])
                .expect("--no-store parses"),
        );
        assert!(opted_out.no_store, "--no-store is captured");
    }

    #[test]
    fn run_parses_the_timeout_and_grace_flags() {
        // `--timeout` and `--grace` capture their duration strings; both default absent.
        let default =
            run_args(Cli::try_parse_from(["tmx", "run", "flow.yaml"]).expect("bare run parses"));
        assert!(
            default.timeout.is_none() && default.grace.is_none(),
            "cancellation flags default off"
        );

        let cancelled = run_args(
            Cli::try_parse_from([
                "tmx",
                "run",
                "flow.yaml",
                "--timeout",
                "30s",
                "--grace",
                "2s",
            ])
            .expect("--timeout/--grace parse"),
        );
        assert_eq!(
            cancelled.timeout.as_deref(),
            Some("30s"),
            "--timeout captures its duration"
        );
        assert_eq!(
            cancelled.grace.as_deref(),
            Some("2s"),
            "--grace captures its duration"
        );

        // Negative space: `--timeout` requires a value — a bare flag is a usage error, not a default.
        assert!(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--timeout"]).is_err(),
            "--timeout without a value is rejected"
        );
    }

    #[test]
    fn runs_parses_each_subcommand() {
        // Each `tmx runs` sub-action parses to its `RunsCommand` variant.
        let list = Cli::try_parse_from(["tmx", "runs", "list"]).expect("runs list parses");
        assert!(
            matches!(
                list.command,
                Command::Runs(RunsArgs {
                    command: RunsCommand::List
                })
            ),
            "list maps to RunsCommand::List"
        );
        let show = Cli::try_parse_from(["tmx", "runs", "show", "abc"]).expect("runs show parses");
        match show.command {
            Command::Runs(RunsArgs {
                command: RunsCommand::Show { id },
            }) => assert_eq!(id, "abc", "show captures the id"),
            other => panic!("expected runs show, got {other:?}"),
        }
        let prune = Cli::try_parse_from(["tmx", "runs", "prune"]).expect("runs prune parses");
        assert!(
            matches!(
                prune.command,
                Command::Runs(RunsArgs {
                    command: RunsCommand::Prune
                })
            ),
            "prune maps to RunsCommand::Prune"
        );
        // Negative space: `show` requires an id, and an unknown sub-action is rejected.
        assert!(
            Cli::try_parse_from(["tmx", "runs", "show"]).is_err(),
            "show without an id is a usage error"
        );
        assert!(
            Cli::try_parse_from(["tmx", "runs", "teleport"]).is_err(),
            "an unknown runs sub-action is rejected"
        );
    }

    #[test]
    fn run_parses_the_check_produces_flag_in_its_three_states() {
        // Absent → None (off); bare `--check-produces` → warn; `--check-produces=strict` → strict.
        let absent =
            run_args(Cli::try_parse_from(["tmx", "run", "flow.yaml"]).expect("bare run parses"));
        assert!(
            absent.check_produces.is_none(),
            "an absent flag leaves the check off"
        );

        let bare = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--check-produces"])
                .expect("bare --check-produces parses"),
        );
        assert_eq!(
            bare.check_produces,
            Some(CheckProducesArg::Warn),
            "a bare --check-produces defaults to warn"
        );
        assert_eq!(
            bare.check_produces.map(CheckProducesArg::to_check),
            Some(tmx_core::ProducesCheck::Warn),
            "warn maps to the engine ProducesCheck::Warn"
        );

        let strict = run_args(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--check-produces=strict"])
                .expect("--check-produces=strict parses"),
        );
        assert_eq!(
            strict.check_produces.map(CheckProducesArg::to_check),
            Some(tmx_core::ProducesCheck::Strict),
            "=strict maps to ProducesCheck::Strict"
        );

        // Negative space: an unknown value is a usage error, never a silent default.
        assert!(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--check-produces=loose"]).is_err(),
            "an unknown --check-produces value is rejected"
        );
    }

    #[test]
    fn run_parses_the_full_flag_depth_surface() {
        // The depth flags (inputs, env, state, slicing, dry-run, matrix, concurrency, …) all parse.
        let args = run_args(
            Cli::try_parse_from([
                "tmx",
                "run",
                "flow.yaml",
                "--input",
                "count=3",
                "-i",
                "flag:=true",
                "--inputs-file",
                "inputs.json",
                "--env",
                "TOKEN=abc",
                "--state-in",
                "prior.json",
                "--state-out",
                "final.json",
                "--from",
                "build",
                "--until",
                "test",
                "--only",
                "build",
                "--skip",
                "lint",
                "--dry-run",
                "--matrix",
                "os=linux,mac",
                "--matrix",
                "arch=x64,arm",
                "--concurrency",
                "4",
                "--continue-on-error",
                "--max-state-size",
                "1048576",
                "--watch",
            ])
            .expect("the full depth surface parses"),
        );
        assert_eq!(
            args.input,
            vec!["count=3".to_string(), "flag:=true".to_string()],
            "--input is repeatable and -i is its short"
        );
        assert_eq!(
            args.inputs_file.as_deref(),
            Some("inputs.json"),
            "--inputs-file"
        );
        assert_eq!(args.env, vec!["TOKEN=abc".to_string()], "--env captured");
        assert_eq!(args.state_in.as_deref(), Some("prior.json"), "--state-in");
        assert_eq!(args.state_out.as_deref(), Some("final.json"), "--state-out");
        assert_eq!(args.from.as_deref(), Some("build"), "--from");
        assert_eq!(args.until.as_deref(), Some("test"), "--until");
        assert_eq!(args.only, vec!["build".to_string()], "--only");
        assert_eq!(args.skip, vec!["lint".to_string()], "--skip");
        assert!(args.dry_run, "--dry-run set");
        assert_eq!(
            args.matrix,
            vec!["os=linux,mac".to_string(), "arch=x64,arm".to_string()],
            "--matrix is a repeatable axis list"
        );
        assert_eq!(args.concurrency, Some(4), "--concurrency captured");
        assert!(args.continue_on_error, "--continue-on-error set");
        assert_eq!(
            args.max_state_size,
            Some(1_048_576),
            "--max-state-size captured"
        );
        assert!(args.watch, "--watch set");

        // Defaults: a bare run leaves every depth flag empty/off.
        let bare = run_args(Cli::try_parse_from(["tmx", "run", "flow.yaml"]).expect("bare run"));
        assert!(
            bare.input.is_empty() && bare.matrix.is_empty() && !bare.dry_run && !bare.watch,
            "the depth flags default off"
        );

        // Negative space: `-n` is the `--dry-run` short, and a non-numeric --concurrency is rejected.
        let short =
            run_args(Cli::try_parse_from(["tmx", "run", "flow.yaml", "-n"]).expect("-n parses"));
        assert!(short.dry_run, "-n is the --dry-run short");
        assert!(
            Cli::try_parse_from(["tmx", "run", "flow.yaml", "--concurrency", "lots"]).is_err(),
            "a non-numeric --concurrency is a usage error"
        );
    }

    #[test]
    fn lint_parses_a_flow_and_the_strict_flag() {
        // `tmx lint flow.yaml --strict` captures the flow and sets strict; strict defaults off.
        let cli = Cli::try_parse_from(["tmx", "lint", "flow.yaml", "--strict"])
            .expect("lint --strict parses");
        match cli.command {
            Command::Lint(args) => {
                assert_eq!(
                    args.flow.as_deref(),
                    Some("flow.yaml"),
                    "the flow is captured"
                );
                assert!(args.strict, "--strict is set");
            }
            other => panic!("expected a lint command, got {other:?}"),
        }

        let plain = Cli::try_parse_from(["tmx", "lint"]).expect("bare lint parses");
        match plain.command {
            Command::Lint(args) => {
                assert!(args.flow.is_none(), "no positional flow");
                assert!(!args.strict, "strict defaults off");
            }
            other => panic!("expected a lint command, got {other:?}"),
        }
    }

    #[test]
    fn env_parses_a_method_and_a_flow() {
        // `tmx env deploy flow.yaml` selects the deploy method against the flow; up/down aggregate.
        let cli =
            Cli::try_parse_from(["tmx", "env", "deploy", "flow.yaml"]).expect("env deploy parses");
        match cli.command {
            Command::Env(args) => {
                assert_eq!(
                    args.method,
                    EnvMethod::Deploy,
                    "the deploy method is selected"
                );
                assert_eq!(
                    args.flow.as_deref(),
                    Some("flow.yaml"),
                    "the flow is captured"
                );
            }
            other => panic!("expected an env command, got {other:?}"),
        }

        let up = Cli::try_parse_from(["tmx", "env", "up"]).expect("env up parses");
        match up.command {
            Command::Env(args) => {
                assert_eq!(args.method, EnvMethod::Up, "up is an aggregate method")
            }
            other => panic!("expected an env command, got {other:?}"),
        }

        // Negative space: an unknown env method is a clap usage error, not a silent default.
        assert!(
            Cli::try_parse_from(["tmx", "env", "teleport"]).is_err(),
            "an unknown env method is rejected"
        );
    }
}
