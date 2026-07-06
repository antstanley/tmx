//! `tmx context show` — the resolved-context projection (07 §Command mapping: an `InspectFlow`
//! projection).
//!
//! Reuses [`inspect`](crate::commands::inspect)'s masked projection path: it resolves and preflights
//! the Flow, then returns the env-vars + **masked** secrets slice of the same view `tmx inspect`
//! builds — so context and secrets share one masking path, never a second, un-masked one.

use serde_json::{Value, json};

use tmx_core::RunError;

use crate::args::ContextCommand;
use crate::commands::inspect::{context_projection, resolve_and_preflight};

/// Run `tmx context <sub>`, returning the resolved context (env + masked secrets) as one JSON object.
///
/// # Errors
///
/// Returns `resolution` for an unresolved Flow or `validation` (exit 3) for a malformed artifact,
/// fail-fast before any projection is printed.
pub async fn execute(args: crate::args::ContextArgs) -> Result<Value, RunError> {
    match args.command {
        ContextCommand::Show { flow, file } => {
            let preflighted = resolve_and_preflight(flow.as_deref(), file.as_deref()).await?;
            Ok(json!({
                "flow": preflighted.flow.name.clone(),
                "context": context_projection(preflighted.flow.context.as_ref()),
            }))
        }
    }
}
