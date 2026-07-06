//! `tmx version` — the CLI-local version command (07 §Command mapping: no use case).
//!
//! Prints the CLI's own version and the TMX spec version this build implements
//! ([`SUPPORTED_SPEC_VERSION`](tmx_schema::SUPPORTED_SPEC_VERSION)) as one JSON object, so a consumer
//! can pin both. `tmx --version` (clap's built-in) prints the crate version alone; `tmx version`
//! additionally reports the supported spec version.

use serde_json::{Value, json};

/// The version projection: the CLI version and the supported TMX spec version.
#[must_use]
pub fn execute() -> Value {
    json!({
        "cli": env!("CARGO_PKG_VERSION"),
        "spec": tmx_schema::SUPPORTED_SPEC_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_reports_the_cli_and_spec_versions() {
        let view = execute();
        assert_eq!(
            view["cli"],
            json!(env!("CARGO_PKG_VERSION")),
            "the CLI version is reported"
        );
        assert_eq!(
            view["spec"],
            json!(tmx_schema::SUPPORTED_SPEC_VERSION),
            "the supported spec version is reported"
        );
    }
}
