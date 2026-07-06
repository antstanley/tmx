//! Argument parsing — the `clap` surface for the `tmx` binary (07 §Implementation layout).
//!
//! The CLI is hybrid (07 §Command → use case mapping): flat primaries for high-frequency actions and
//! noun groups for resource areas. Task 17 lands the load-bearing primary, [`Command::Run`]; the rest
//! of the surface arrives with its own tasks. This module owns only the *shape* of the arguments —
//! `main` maps a parsed command to a use case and its result to an exit code.

use clap::{Parser, Subcommand};

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

/// The top-level commands. Task 17 implements `run`; the others land with their tasks.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a Flow end to end: load, preflight, execute, and print the masked final state to stdout.
    Run(RunArgs),
}

/// Arguments for `tmx run` — the core surface task 17 needs (07 §`tmx run`). The full run-flag depth
/// (inputs, env, slicing, dry-run, matrix, concurrency, …) is task 30.
#[derive(Debug, Default, Parser)]
pub struct RunArgs {
    /// The Flow to run: a file (`flow.yaml`/`.yml`/`.json`/`.jsonc`/`.toml`) or a directory layout.
    /// When omitted, the flow is resolved by the search order (see the resolver).
    pub flow: Option<String>,

    /// Explicit Flow file/directory, taking precedence over the positional argument and `$TMX_FLOW`.
    #[arg(short = 'f', long = "file")]
    pub file: Option<String>,
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

    #[test]
    fn run_parses_positional_and_file_flag() {
        // The positional flow and the -f/--file override both parse into RunArgs.
        let cli = Cli::try_parse_from(["tmx", "run", "pipeline.yaml"]).expect("positional parses");
        let Command::Run(args) = cli.command;
        assert_eq!(
            args.flow.as_deref(),
            Some("pipeline.yaml"),
            "positional flow"
        );
        assert!(args.file.is_none(), "no --file given");

        let cli = Cli::try_parse_from(["tmx", "run", "--file", "explicit.toml"])
            .expect("the --file flag parses");
        let Command::Run(args) = cli.command;
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
}
