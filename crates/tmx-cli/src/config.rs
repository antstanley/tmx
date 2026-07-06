//! Configuration resolution — the layered effective config the composition root reads
//! (07 §Configuration).
//!
//! The full layering (CLI flags → `TMX_*` env → project/user/system config files → named profiles) is
//! elaborated by later tasks. Task 17 lands the one layer `tmx run` needs to resolve a Flow: the
//! `$TMX_FLOW` environment fallback, plus the conventional filename candidates the cwd search probes
//! (`./flow.{…}` then `./tmx.{…}`). Keeping these here — not hard-coded in the run command — is what
//! lets later tasks widen the search order and the config layers in one place.

use tmx_adapters::sink::Format;

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

/// Resolve the stdout reporter [`Format`] by the documented precedence (07 §Configuration): the
/// `--format` flag, then `TMX_FORMAT`, then the TTY default (`pretty` when stdout is an interactive
/// terminal, `json` when it is a pipe/file). An unknown `TMX_FORMAT` token is ignored (it falls
/// through to the TTY default) rather than aborting the run — a bad flag is clap's exit-2 job, but a
/// stray env var should not.
#[must_use]
pub fn resolve_format(flag: Option<Format>, stdout_is_tty: bool) -> Format {
    let env_format = std::env::var("TMX_FORMAT")
        .ok()
        .and_then(|token| Format::parse(&token));
    resolve_format_with(flag, env_format, stdout_is_tty)
}

/// The env-free core of [`resolve_format`]: the flag wins, then a resolved `TMX_FORMAT`, then the TTY
/// default. Split out so the precedence is tested without touching process env.
#[must_use]
fn resolve_format_with(
    flag: Option<Format>,
    env_format: Option<Format>,
    stdout_is_tty: bool,
) -> Format {
    flag.or(env_format)
        .unwrap_or_else(|| Format::default_for_tty(stdout_is_tty))
}

/// Resolve whether the stderr progress is ANSI-coloured (07 §Configuration; steps §`NO_COLOR`).
/// Precedence: `--no-color` (or `NO_COLOR` / `TMX_NO_COLOR` present) forces colour **off**; `--color`
/// forces it **on**; otherwise colour follows the stderr TTY check. `NO_COLOR` follows the informal
/// standard — its mere presence (even empty) disables colour.
#[must_use]
pub fn resolve_color(color_flag: bool, no_color_flag: bool, stderr_is_tty: bool) -> bool {
    let env_disables = std::env::var_os("NO_COLOR").is_some()
        || std::env::var_os("TMX_NO_COLOR").is_some_and(|v| !v.is_empty());
    resolve_color_with(color_flag, no_color_flag, env_disables, stderr_is_tty)
}

/// The env-free core of [`resolve_color`]: `--no-color` / an env disable wins (off), then `--color`
/// (on), then the stderr TTY check. Split out so the precedence is tested without touching process env.
#[must_use]
fn resolve_color_with(
    color_flag: bool,
    no_color_flag: bool,
    env_disables: bool,
    stderr_is_tty: bool,
) -> bool {
    if no_color_flag || env_disables {
        return false;
    }
    if color_flag {
        return true;
    }
    stderr_is_tty
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
    fn format_precedence_flag_then_env_then_tty_default() {
        // The flag wins over both a resolved TMX_FORMAT and the TTY default.
        assert_eq!(
            resolve_format_with(Some(Format::Ndjson), Some(Format::Json), true),
            Format::Ndjson,
            "the --format flag wins over env and the TTY default"
        );
        // With no flag, a resolved TMX_FORMAT wins over the TTY default.
        assert_eq!(
            resolve_format_with(None, Some(Format::Ndjson), true),
            Format::Ndjson,
            "TMX_FORMAT wins over the TTY default when no flag is given"
        );
        // With neither, the TTY default decides: pretty at a terminal, json when piped.
        assert_eq!(
            resolve_format_with(None, None, true),
            Format::Pretty,
            "a bare interactive run defaults to pretty"
        );
        assert_eq!(
            resolve_format_with(None, None, false),
            Format::Json,
            "a bare piped run defaults to json so `| jq` works"
        );
    }

    #[test]
    fn color_precedence_off_wins_then_on_then_tty() {
        // --no-color and an env disable both force colour off, even over --color and a TTY.
        assert!(
            !resolve_color_with(true, true, false, true),
            "--no-color wins over --color"
        );
        assert!(
            !resolve_color_with(true, false, true, true),
            "an env disable (NO_COLOR) forces colour off"
        );
        // --color forces colour on when nothing disables it, even off a TTY.
        assert!(
            resolve_color_with(true, false, false, false),
            "--color forces colour on"
        );
        // With no flags, colour follows the stderr TTY check.
        assert!(
            resolve_color_with(false, false, false, true),
            "a colour-capable TTY defaults colour on"
        );
        assert!(
            !resolve_color_with(false, false, false, false),
            "a piped stderr defaults colour off"
        );
    }

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
