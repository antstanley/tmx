//! Configuration resolution — the layered effective config the composition root reads
//! (07 §Configuration).
//!
//! The full layering (CLI flags → `TMX_*` env → project/user/system config files → named profiles) is
//! elaborated by later tasks. Task 17 lands the one layer `tmx run` needs to resolve a Flow: the
//! `$TMX_FLOW` environment fallback, plus the conventional filename candidates the cwd search probes
//! (`./flow.{…}` then `./tmx.{…}`). Keeping these here — not hard-coded in the run command — is what
//! lets later tasks widen the search order and the config layers in one place.

/// The reserved Flow-file stems the cwd search probes, in precedence order: `flow.*` before `tmx.*`
/// (07 §`tmx run`).
pub const FLOW_STEMS: [&str; 2] = ["flow", "tmx"];

/// The source extensions a Flow file may carry, in precedence order — the four wire formats plus the
/// YAML `.yml` alias (03 §Source loading). A units-last-style ordered vocabulary, not a numeric limit.
pub const FLOW_EXTENSIONS: [&str; 5] = ["yaml", "yml", "json", "jsonc", "toml"];

/// The `$TMX_FLOW` environment fallback — the third rung of the resolution order, after `--file` and
/// the positional argument (07 §`tmx run`). `None` when the variable is unset or empty.
#[must_use]
pub fn env_flow() -> Option<String> {
    std::env::var("TMX_FLOW")
        .ok()
        .filter(|value| !value.is_empty())
}

/// The conventional Flow-file names the cwd search probes, in resolution order: every extension of
/// `flow.*`, then every extension of `tmx.*` (07 §`tmx run`).
#[must_use]
pub fn flow_file_candidates() -> Vec<String> {
    let mut candidates = Vec::with_capacity(FLOW_STEMS.len() * FLOW_EXTENSIONS.len());
    for stem in FLOW_STEMS {
        for ext in FLOW_EXTENSIONS {
            candidates.push(format!("{stem}.{ext}"));
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_file_candidates_are_ordered_flow_before_tmx() {
        let candidates = flow_file_candidates();
        assert_eq!(
            candidates.len(),
            FLOW_STEMS.len() * FLOW_EXTENSIONS.len(),
            "one candidate per stem×extension"
        );
        assert_eq!(
            candidates.first().map(String::as_str),
            Some("flow.yaml"),
            "flow.yaml is probed first"
        );
        // `flow.*` fully precedes `tmx.*` — the last flow candidate comes before the first tmx one.
        let first_tmx = candidates
            .iter()
            .position(|c| c.starts_with("tmx."))
            .expect("a tmx candidate exists");
        let last_flow = candidates
            .iter()
            .rposition(|c| c.starts_with("flow."))
            .expect("a flow candidate exists");
        assert!(last_flow < first_tmx, "every flow.* precedes every tmx.*");
    }
}
