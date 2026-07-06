//! `tmx secrets list` — the needed-secrets projection (07 §Command mapping: an `InspectFlow`
//! projection with masked values).
//!
//! Reuses [`inspect`](crate::commands::inspect)'s masked projection path: it resolves and preflights
//! the Flow, then returns the secrets-needed slice — one entry per declared secret, naming the
//! secret and its source kind, with the value **masked**. It never prints a raw secret value; the
//! same masking path backs `tmx context show`.

use serde_json::{Value, json};

use tmx_core::RunError;

use crate::args::SecretsCommand;
use crate::commands::inspect::{resolve_and_preflight, secrets_needed};

/// Run `tmx secrets <sub>`, returning the masked needed-secrets listing as one JSON object.
///
/// # Errors
///
/// Returns `resolution` for an unresolved Flow or `validation` (exit 3) for a malformed artifact,
/// fail-fast before any projection is printed.
pub async fn execute(args: crate::args::SecretsArgs) -> Result<Value, RunError> {
    match args.command {
        SecretsCommand::List { flow, file } => {
            let preflighted = resolve_and_preflight(flow.as_deref(), file.as_deref()).await?;
            Ok(json!({
                "flow": preflighted.flow.name.clone(),
                "secretsNeeded": secrets_needed(preflighted.flow.context.as_ref()),
            }))
        }
    }
}
