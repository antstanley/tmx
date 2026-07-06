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
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Command,
}

/// The top-level commands. Task 17 implements `run`; task 25 adds `env` (provider lifecycle).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a Flow end to end: load, preflight, execute, and print the masked final state to stdout.
    Run(RunArgs),
    /// Drive a Flow's environment provider through its lifecycle methods (07 §`tmx env`).
    Env(EnvArgs),
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
