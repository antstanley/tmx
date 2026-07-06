//! Configuration resolution — the layered effective config the composition root reads
//! (07 §Configuration).
//!
//! The full layering (CLI flags → `TMX_*` env → project/user/system config files → named profiles) is
//! elaborated by later tasks. Task 17 lands the one layer `tmx run` needs to resolve a Flow: the
//! `$TMX_FLOW` environment fallback, plus the conventional filename candidates the cwd search probes
//! (`./flow.{…}` then `./tmx.{…}`). Keeping these here — not hard-coded in the run command — is what
//! lets later tasks widen the search order and the config layers in one place.

use tmx_adapters::sink::Format;
use tmx_schema::limits::{CANCEL_GRACE_MS, RUN_RETENTION_DEFAULT_DAYS};

/// Milliseconds per second — the duration-suffix conversion factor for [`parse_duration_ms`].
const MILLISECONDS_PER_SECOND: u64 = 1000;
/// Milliseconds per minute.
const MILLISECONDS_PER_MINUTE: u64 = 60 * MILLISECONDS_PER_SECOND;
/// Milliseconds per hour.
const MILLISECONDS_PER_HOUR: u64 = 60 * MILLISECONDS_PER_MINUTE;

/// Parse a `--timeout`/`--grace` duration string (`500ms`/`30s`/`5m`/`1h`, or a bare integer read as
/// seconds) into whole milliseconds. Returns `None` for an unparseable value, so the caller can reject
/// or fall back rather than silently mis-timing a cancellation. Mirrors the runtime `duration` grammar
/// the task-level `timeout` fields use, kept here so the flag layer parses the same forms.
#[must_use]
pub fn parse_duration_ms(raw: &str) -> Option<u64> {
    let spec = raw.trim();
    if spec.is_empty() {
        return None;
    }
    let (number, unit_ms) = if let Some(rest) = spec.strip_suffix("ms") {
        (rest, 1)
    } else if let Some(rest) = spec.strip_suffix('s') {
        (rest, MILLISECONDS_PER_SECOND)
    } else if let Some(rest) = spec.strip_suffix('m') {
        (rest, MILLISECONDS_PER_MINUTE)
    } else if let Some(rest) = spec.strip_suffix('h') {
        (rest, MILLISECONDS_PER_HOUR)
    } else {
        (spec, MILLISECONDS_PER_SECOND)
    };
    number
        .trim()
        .parse::<u64>()
        .ok()
        .map(|value| value.saturating_mul(unit_ms))
}

/// Resolve the cancellation grace window in milliseconds: the `--grace` flag when it parses, else the
/// [`CANCEL_GRACE_MS`] default. `--grace 0` is honoured (an immediate hard stop); an unparseable value
/// falls back to the default rather than aborting the run over a stray flag value.
#[must_use]
pub fn resolve_grace_ms(grace_flag: Option<&str>) -> u64 {
    grace_flag
        .and_then(parse_duration_ms)
        .unwrap_or(CANCEL_GRACE_MS)
}

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

/// Resolve the run-store retention window, in whole days: `Some(n)` retains runs for `n` days;
/// `None` disables the retention sweep entirely (08 §Run store). Precedence: `TMX_RUNS_RETENTION`
/// (`0` / `off`, case-insensitively, disables; a positive integer sets the window; anything else
/// falls through), else the [`RUN_RETENTION_DEFAULT_DAYS`] default. The `runs.retention` config key
/// joins this precedence when the config-file layer lands.
#[must_use]
pub fn resolve_retention_days() -> Option<u64> {
    match std::env::var("TMX_RUNS_RETENTION") {
        Ok(raw) => resolve_retention_with(&raw),
        Err(_) => Some(RUN_RETENTION_DEFAULT_DAYS),
    }
}

/// The env-free core of [`resolve_retention_days`]: `0` / `off` disables (`None`), a positive integer
/// sets the window, and an unparseable token falls back to the default rather than aborting the run
/// (a stray env var must not fail a run, mirroring the `TMX_FORMAT` handling). Split out so the
/// precedence is tested without touching process env.
#[must_use]
fn resolve_retention_with(raw: &str) -> Option<u64> {
    let token = raw.trim();
    if token.eq_ignore_ascii_case("off") || token == "0" {
        return None;
    }
    match token.parse::<u64>() {
        Ok(days) if days > 0 => Some(days),
        // An unparseable / zero-after-parse token falls back to the default window.
        _ => Some(RUN_RETENTION_DEFAULT_DAYS),
    }
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

// =================================================================================================
// Layered configuration (07 §Configuration).
//
// The effective config resolves highest-to-lowest: CLI flags > `TMX_*` env > project `tmx.config.*`
// > user (`~/.config/tmx/config.toml`) > system (`/etc/tmx/config.toml`). Each layer is a flat map
// of config keys to JSON values; a higher layer's key wins. A project layer may carry named
// `profiles` (an active profile's overrides merge above the base project layer) and a `names` map
// of registered-name → path mappings the resolver exposes. `config.rs` resolves the layers; the
// composition root (`compose.rs` / the command modules) consumes the one effective config.
// =================================================================================================

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use tmx_adapters::loader::{detect_source_kind, parse_source};

/// One configuration layer — a flat map of config keys to JSON values (07 §Configuration).
pub type ConfigLayer = Map<String, Value>;

/// The reserved config key under which a project layer nests its named profiles.
const PROFILES_KEY: &str = "profiles";
/// The reserved config key under which a layer nests its registered-name → path mappings.
const NAMES_KEY: &str = "names";

/// The config-file stems probed for each config layer, in the same four wire formats a Flow uses
/// (07 §Configuration: `tmx.config.{toml,yaml,json,jsonc}`), plus the `.yml` alias.
const CONFIG_STEM: &str = "tmx.config";

/// The resolved, layered effective configuration the composition root reads (07 §Configuration).
///
/// Built by [`resolve_effective`] (pure, layer-folding) or [`load_effective`] (which reads the env
/// and the on-disk config files first). A key present in a higher layer shadows the same key in
/// every lower one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectiveConfig {
    values: ConfigLayer,
}

impl EffectiveConfig {
    /// The resolved value for `key` as a string, when it resolved to a JSON string.
    ///
    /// A general accessor the composition root and the run-flag resolution (task 30) read a scalar
    /// config default (`format`, `concurrency`, …) through; provided ahead of that consumer here so
    /// the layered config surface is complete in one place, mirroring the scheduler seam composed
    /// ahead of its fan-out consumer in `compose.rs`.
    #[allow(dead_code)]
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(Value::as_str)
    }

    /// The raw resolved [`Value`] for `key`, when any layer set it — the accessor the numeric run-flag
    /// resolvers ([`resolve_concurrency`]/[`resolve_max_state_size`]) read through, so a value that
    /// arrived as a JSON number (a config file) or a string (a `TMX_*` env var) is both handled.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    /// The path a registered `name` maps to (the `names` layer), when the config declares one — the
    /// registered-name → path mapping `tmx run <name>` and the resource commands consult.
    #[must_use]
    pub fn registered_path(&self, name: &str) -> Option<&str> {
        self.values
            .get(NAMES_KEY)
            .and_then(Value::as_object)
            .and_then(|names| names.get(name))
            .and_then(Value::as_str)
    }

    /// The full registered-name → path map (`names`), empty when none is declared.
    #[must_use]
    pub fn registered_names(&self) -> ConfigLayer {
        self.values
            .get(NAMES_KEY)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }
}

/// Fold `layers` (ordered **highest priority first**) into one effective config: for each key, the
/// first (highest) layer that sets it wins. Pure — the disk/env-free core [`load_effective`] drives.
#[must_use]
pub fn resolve_effective(highest_to_lowest: &[ConfigLayer]) -> EffectiveConfig {
    let mut values = ConfigLayer::new();
    // Walk lowest-to-highest so a higher layer overwrites a lower one, leaving the highest as winner.
    for layer in highest_to_lowest.iter().rev() {
        for (key, value) in layer {
            values.insert(key.clone(), value.clone());
        }
    }
    EffectiveConfig { values }
}

/// Fold an active profile into a project layer: the base keys, with the named profile's overrides
/// merged on top (the profile wins), and the reserved `profiles` key dropped from the result. An
/// absent/unknown profile yields the base layer unchanged (minus `profiles`). Pure.
#[must_use]
pub fn apply_profile(project: &ConfigLayer, profile: Option<&str>) -> ConfigLayer {
    let mut base: ConfigLayer = project
        .iter()
        .filter(|(key, _)| key.as_str() != PROFILES_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let Some(profile) = profile else {
        return base;
    };
    if let Some(overrides) = project
        .get(PROFILES_KEY)
        .and_then(Value::as_object)
        .and_then(|profiles| profiles.get(profile))
        .and_then(Value::as_object)
    {
        for (key, value) in overrides {
            base.insert(key.clone(), value.clone());
        }
    }
    base
}

/// Build the `TMX_*` environment config layer (07 §Configuration): the documented env vars mapped to
/// their config keys. An unset variable contributes no key, so it never shadows a lower layer.
#[must_use]
pub fn env_layer() -> ConfigLayer {
    let mut layer = ConfigLayer::new();
    let mappings = [
        ("TMX_FORMAT", "format"),
        ("TMX_CONCURRENCY", "concurrency"),
        ("TMX_PROFILE", "profile"),
        ("TMX_NO_COLOR", "noColor"),
        ("TMX_MAX_STATE_SIZE", "maxStateSize"),
        ("TMX_RUNS_RETENTION", "runsRetention"),
    ];
    for (var, key) in mappings {
        if let Ok(value) = std::env::var(var)
            && !value.is_empty()
        {
            layer.insert(key.to_string(), Value::String(value));
        }
    }
    layer
}

/// Read a single config layer from the first `tmx.config.*` file present in `dir`, in the same
/// format-precedence order a Flow uses. A missing directory / absent file is an empty layer; a
/// malformed config file is an empty layer (a stray config must not abort every command — the
/// authoritative validation path is `tmx validate`), never a panic.
#[must_use]
pub fn file_layer(dir: &Path) -> ConfigLayer {
    for ext in FLOW_EXTENSIONS {
        let path = dir.join(format!("{CONFIG_STEM}.{ext}"));
        if !path.is_file() {
            continue;
        }
        let Some(text) = std::fs::read_to_string(&path).ok() else {
            continue;
        };
        let Some(path_str) = path.to_str() else {
            continue;
        };
        let Ok(kind) = detect_source_kind(path_str) else {
            continue;
        };
        if let Ok(Value::Object(map)) = parse_source(&text, kind) {
            return map;
        }
        return ConfigLayer::new();
    }
    ConfigLayer::new()
}

/// Resolve the active profile name by the documented precedence: the `--profile` flag layer, then
/// `TMX_PROFILE`, then a `profile` key set in the project file (07 §Configuration). Pure over its
/// layers.
#[must_use]
pub fn active_profile(
    flags: &ConfigLayer,
    env: &ConfigLayer,
    project: &ConfigLayer,
) -> Option<String> {
    for layer in [flags, env, project] {
        if let Some(profile) = layer.get("profile").and_then(Value::as_str) {
            return Some(profile.to_string());
        }
    }
    None
}

/// Load the effective config from `flags` plus the on-disk layers (07 §Configuration): flags >
/// `TMX_*` env > project `tmx.config.*` (`project_dir`) > user (`~/.config/tmx/`) > system
/// (`/etc/tmx/`), with the active profile folded into the project layer.
#[must_use]
pub fn load_effective(flags: ConfigLayer, project_dir: &Path) -> EffectiveConfig {
    let env = env_layer();
    let project_raw = file_layer(project_dir);
    let user = user_config_dir()
        .map(|d| file_layer(&d))
        .unwrap_or_default();
    let system = file_layer(Path::new("/etc/tmx"));
    let profile = active_profile(&flags, &env, &project_raw);
    let project = apply_profile(&project_raw, profile.as_deref());
    resolve_effective(&[flags, env, project, user, system])
}

/// The user config directory (`~/.config/tmx`), when a home directory is known.
#[must_use]
fn user_config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("tmx"))
}

/// Resolve a possibly-registered `reference` against the effective config rooted at `project_dir`
/// (with `flags` folded in): when `reference` names a registered-name → path mapping, return its
/// mapped path; otherwise return `reference` unchanged. This realises the registered-name layer of
/// the config (07 §Configuration) for the resource commands' Flow resolution.
#[must_use]
pub fn resolve_registered(flags: ConfigLayer, project_dir: &Path, reference: &str) -> String {
    load_effective(flags, project_dir)
        .registered_path(reference)
        .map_or_else(|| reference.to_string(), str::to_string)
}

// =================================================================================================
// Layered run overrides (07 §Configuration, §Run flags).
//
// The `--concurrency`/`--max-state-size` caps and the `local` default resolve through the full
// documented precedence — `flag > TMX_CONCURRENCY|TMX_MAX_STATE_SIZE|TMX_NO_ENV > project > user >
// system > built-in default` — with `--profile` selecting a project-config profile. The `tmx run`
// path builds an [`EffectiveConfig`] with the flag values folded in as the highest layer, then reads
// the resolved caps back through these resolvers. A present-but-malformed numeric value (an env var
// or config key that is not a non-negative integer) is a **usage error** (07 §Exit codes: exit 2),
// surfaced before the run starts rather than silently ignored.
// =================================================================================================

/// The layered run overrides the `tmx run` path resolves before executing (07 §Configuration): the
/// `concurrency`/`max-state-size` caps (`None` = the engine default) and whether the run is `local`
/// (no provider lifecycle) after folding `TMX_NO_ENV`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOverrides {
    /// The resolved global `map`/`eval` fan-out concurrency cap, or `None` for the engine default.
    pub concurrency: Option<u32>,
    /// The resolved narrowed state-size cap in bytes, or `None` for the engine default.
    pub max_state_size_bytes: Option<u64>,
    /// Whether the run executes locally with no provider lifecycle (`--local`/`--no-env`/`TMX_NO_ENV`).
    pub local: bool,
}

/// A malformed `TMX_*` / config numeric value — a usage error (07 §Exit codes: exit 2). Carried as a
/// distinct type (not a core [`RunError`]) because exit 2 is CLI-local, not a core error category: the
/// binary maps this straight to exit 2 the same way `clap` maps a bad flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigUsageError {
    /// The human-facing diagnostic naming the offending variable and value.
    pub message: String,
}

impl std::fmt::Display for ConfigUsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Build a [`ConfigUsageError`] naming the source variable and the offending `value`.
fn malformed_numeric(source: &str, value: &str) -> ConfigUsageError {
    ConfigUsageError {
        message: format!("{source} must be a non-negative integer, got {value:?}"),
    }
}

/// Read a resolved config `key` as a `u64`, accepting either a JSON number (a config file) or a numeric
/// string (a `TMX_*` env var). `Ok(None)` when no layer set it; `Err` (naming `source`) when a layer
/// set it to a value that is not a non-negative integer, so a stray value is a usage error rather than
/// a silent fall-back to the default.
fn resolve_u64_key(
    effective: &EffectiveConfig,
    key: &str,
    source: &str,
) -> Result<Option<u64>, ConfigUsageError> {
    match effective.get(key) {
        None => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| malformed_numeric(source, &number.to_string())),
        Some(Value::String(text)) => text
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| malformed_numeric(source, text)),
        Some(other) => Err(malformed_numeric(source, &other.to_string())),
    }
}

/// Resolve the effective `concurrency` cap as a `u32` from the layered config (`flag > TMX_CONCURRENCY
/// > project > user > system`). `Ok(None)` leaves the engine ceiling as the only bound; a present
/// value that is not a `u32` (non-numeric, negative, or beyond `u32`) is a usage error (exit 2). A
/// value within `u32` but above the engine ceiling is *not* rejected here — the run's own
/// `check_concurrency` maps that to a validation error (exit 3), keeping the two failure modes distinct.
///
/// # Errors
///
/// Returns [`ConfigUsageError`] when `TMX_CONCURRENCY` or a `concurrency` config key is set to a
/// non-`u32` value.
pub fn resolve_concurrency(effective: &EffectiveConfig) -> Result<Option<u32>, ConfigUsageError> {
    let source = "TMX_CONCURRENCY / concurrency";
    match resolve_u64_key(effective, "concurrency", source)? {
        None => Ok(None),
        Some(value) => u32::try_from(value)
            .map(Some)
            .map_err(|_| malformed_numeric(source, &value.to_string())),
    }
}

/// Resolve the effective `maxStateSize` cap in bytes from the layered config (`flag >
/// TMX_MAX_STATE_SIZE > project > user > system`). `Ok(None)` leaves the engine default; the runner
/// clamps a resolved value to the hard `STATE_SIZE_MAX_BYTES` ceiling. A present non-integer value is
/// a usage error (exit 2).
///
/// # Errors
///
/// Returns [`ConfigUsageError`] when `TMX_MAX_STATE_SIZE` or a `maxStateSize` config key is set to a
/// non-integer value.
pub fn resolve_max_state_size(
    effective: &EffectiveConfig,
) -> Result<Option<u64>, ConfigUsageError> {
    resolve_u64_key(
        effective,
        "maxStateSize",
        "TMX_MAX_STATE_SIZE / maxStateSize",
    )
}

/// Resolve whether the run executes `local` (no provider lifecycle): the `--local`/`--no-env` flag, or
/// `TMX_NO_ENV` present and non-empty (07 §Configuration). An explicit flag still wins — a set flag is
/// `true` regardless of the env — so this only *adds* the env as an equivalent trigger.
#[must_use]
pub fn resolve_local(local_flag: bool) -> bool {
    let env_no_env = std::env::var_os("TMX_NO_ENV").is_some_and(|value| !value.is_empty());
    resolve_local_with(local_flag, env_no_env)
}

/// The env-free core of [`resolve_local`]: the flag OR the `TMX_NO_ENV` presence. Split out so the
/// precedence is tested without touching process env.
#[must_use]
fn resolve_local_with(local_flag: bool, env_no_env: bool) -> bool {
    local_flag || env_no_env
}

#[cfg(test)]
mod run_override_tests {
    use super::*;
    use serde_json::json;

    /// Build an [`EffectiveConfig`] from a single JSON-object layer.
    fn effective(value: Value) -> EffectiveConfig {
        let layer = value.as_object().cloned().expect("an object literal");
        resolve_effective(&[layer])
    }

    #[test]
    fn resolve_concurrency_reads_a_number_or_a_numeric_string_and_rejects_garbage() {
        // A JSON number (a config file) and a numeric string (a TMX_* env var) both resolve.
        assert_eq!(
            resolve_concurrency(&effective(json!({ "concurrency": 4 })))
                .expect("a numeric concurrency resolves"),
            Some(4),
            "a JSON number resolves to the cap"
        );
        assert_eq!(
            resolve_concurrency(&effective(json!({ "concurrency": "8" })))
                .expect("a numeric string resolves"),
            Some(8),
            "a numeric string (the env-var form) resolves to the cap"
        );
        // Absent → None (the engine ceiling is the only bound).
        assert_eq!(
            resolve_concurrency(&effective(json!({}))).expect("absent is Ok"),
            None,
            "no concurrency key leaves the cap unset"
        );

        // Negative space: a non-numeric value is a usage error, never a silent fall-back.
        let err = resolve_concurrency(&effective(json!({ "concurrency": "lots" })))
            .expect_err("a non-numeric concurrency is rejected");
        assert!(
            err.message.contains("TMX_CONCURRENCY"),
            "the usage error names the source, got {:?}",
            err.message
        );
        // A value beyond u32 is also a usage error, not a truncation.
        assert!(
            resolve_concurrency(&effective(json!({ "concurrency": 9_999_999_999_u64 }))).is_err(),
            "a value beyond u32 is rejected, not truncated"
        );
    }

    #[test]
    fn resolve_max_state_size_reads_bytes_and_rejects_garbage() {
        assert_eq!(
            resolve_max_state_size(&effective(json!({ "maxStateSize": "1048576" })))
                .expect("a byte count resolves"),
            Some(1_048_576),
            "a numeric string resolves to the byte cap"
        );
        assert_eq!(
            resolve_max_state_size(&effective(json!({}))).expect("absent is Ok"),
            None,
            "no maxStateSize key leaves the cap at the engine default"
        );

        // Negative space: a malformed byte count is a usage error.
        let err = resolve_max_state_size(&effective(json!({ "maxStateSize": "big" })))
            .expect_err("a malformed byte count is rejected");
        assert!(
            err.message.contains("TMX_MAX_STATE_SIZE"),
            "the usage error names the source, got {:?}",
            err.message
        );
    }

    #[test]
    fn resolve_concurrency_honours_the_layer_precedence() {
        // flag (top layer) beats env beats project — the load-bearing precedence for the caps.
        let flag_wins = resolve_effective(&[
            json!({ "concurrency": 2 }).as_object().cloned().unwrap(),
            json!({ "concurrency": "8" }).as_object().cloned().unwrap(),
            json!({ "concurrency": "16" }).as_object().cloned().unwrap(),
        ]);
        assert_eq!(
            resolve_concurrency(&flag_wins).expect("resolves"),
            Some(2),
            "the flag layer wins the concurrency cap"
        );
        let env_wins = resolve_effective(&[
            ConfigLayer::new(),
            json!({ "concurrency": "8" }).as_object().cloned().unwrap(),
            json!({ "concurrency": "16" }).as_object().cloned().unwrap(),
        ]);
        assert_eq!(
            resolve_concurrency(&env_wins).expect("resolves"),
            Some(8),
            "with no flag the env layer wins over the project"
        );
    }

    #[test]
    fn resolve_local_folds_the_flag_and_the_env() {
        // The explicit flag wins; TMX_NO_ENV is an equivalent trigger; neither leaves local off.
        assert!(
            resolve_local_with(true, false),
            "the --local flag forces local"
        );
        assert!(
            resolve_local_with(false, true),
            "TMX_NO_ENV alone triggers local"
        );
        assert!(
            resolve_local_with(true, true),
            "flag and env together still local"
        );
        assert!(
            !resolve_local_with(false, false),
            "neither flag nor env leaves the run non-local"
        );
    }
}

#[cfg(test)]
mod config_layer_tests {
    use super::*;
    use serde_json::json;

    /// Build a config layer from a JSON object literal.
    fn layer(value: Value) -> ConfigLayer {
        value.as_object().cloned().expect("an object literal")
    }

    #[test]
    fn effective_config_resolves_highest_to_lowest() {
        // A key set in several layers is won by the highest; a key set only in a lower layer still
        // resolves. Order: flags > env > project > user > system.
        let flags = layer(json!({ "format": "ndjson" }));
        let env = layer(json!({ "format": "json", "concurrency": "8" }));
        let project = layer(json!({ "format": "pretty", "concurrency": "4", "profile": "ci" }));
        let user = layer(json!({ "retention": "14" }));
        let system = layer(json!({ "retention": "30", "systemOnly": "yes" }));

        let effective = resolve_effective(&[flags, env, project, user, system]);
        assert_eq!(
            effective.get_str("format"),
            Some("ndjson"),
            "the flag layer wins `format` over env, project, and below"
        );
        assert_eq!(
            effective.get_str("concurrency"),
            Some("8"),
            "with no flag, the env layer wins `concurrency` over the project"
        );
        assert_eq!(
            effective.get_str("retention"),
            Some("14"),
            "a key set only below the project resolves from the user layer, not system"
        );
        assert_eq!(
            effective.get_str("systemOnly"),
            Some("yes"),
            "a system-only key still resolves"
        );
    }

    #[test]
    fn a_flag_overrides_an_env_var_which_overrides_the_project_file() {
        // The load-bearing precedence the certificate names: flag > TMX_* env > project tmx.config.*.
        let flag_wins = resolve_effective(&[
            layer(json!({ "format": "ndjson" })),
            layer(json!({ "format": "json" })),
            layer(json!({ "format": "pretty" })),
        ]);
        assert_eq!(
            flag_wins.get_str("format"),
            Some("ndjson"),
            "flag beats all"
        );

        // Drop the flag: the env var now wins over the project file.
        let env_wins = resolve_effective(&[
            ConfigLayer::new(),
            layer(json!({ "format": "json" })),
            layer(json!({ "format": "pretty" })),
        ]);
        assert_eq!(
            env_wins.get_str("format"),
            Some("json"),
            "env beats project"
        );

        // Drop the env layer too: the project file is the resolved value.
        let project_wins = resolve_effective(&[
            ConfigLayer::new(),
            ConfigLayer::new(),
            layer(json!({ "format": "pretty" })),
        ]);
        assert_eq!(
            project_wins.get_str("format"),
            Some("pretty"),
            "the project file resolves when nothing above sets the key"
        );
    }

    #[test]
    fn apply_profile_overrides_base_keys_and_drops_the_profiles_key() {
        // An active profile's keys override the base; the reserved `profiles` key never leaks into
        // the resolved layer.
        let project = layer(json!({
            "format": "pretty",
            "concurrency": "4",
            "profiles": {
                "ci": { "format": "ndjson", "concurrency": "16" },
                "local": { "format": "json" }
            }
        }));
        let ci = apply_profile(&project, Some("ci"));
        assert_eq!(
            ci.get("format").and_then(Value::as_str),
            Some("ndjson"),
            "the ci profile overrides the base format"
        );
        assert_eq!(
            ci.get("concurrency").and_then(Value::as_str),
            Some("16"),
            "the ci profile overrides the base concurrency"
        );
        assert!(
            !ci.contains_key(PROFILES_KEY),
            "the profiles block is stripped from the resolved layer"
        );

        // Negative space: no active profile leaves the base untouched (still minus `profiles`).
        let base = apply_profile(&project, None);
        assert_eq!(
            base.get("format").and_then(Value::as_str),
            Some("pretty"),
            "with no profile the base format stands"
        );
        assert!(!base.contains_key(PROFILES_KEY), "profiles still stripped");

        // An unknown profile also leaves the base standing (no overrides to apply).
        let unknown = apply_profile(&project, Some("nope"));
        assert_eq!(
            unknown.get("format").and_then(Value::as_str),
            Some("pretty"),
            "an unknown profile applies no override"
        );
    }

    #[test]
    fn registered_names_expose_the_name_to_path_mapping() {
        // The `names` layer is projected as registered-name → path lookups.
        let effective = resolve_effective(&[layer(json!({
            "names": { "deploy": "flows/deploy.yaml", "test": "flows/test.toml" }
        }))]);
        assert_eq!(
            effective.registered_path("deploy"),
            Some("flows/deploy.yaml"),
            "a registered name resolves to its path"
        );
        assert!(
            effective.registered_path("absent").is_none(),
            "an unregistered name resolves to nothing"
        );
        assert_eq!(
            effective.registered_names().len(),
            2,
            "both names are exposed"
        );
    }

    #[test]
    fn active_profile_resolves_flag_then_env_then_project() {
        let flag = layer(json!({ "profile": "flagprof" }));
        let env = layer(json!({ "profile": "envprof" }));
        let project = layer(json!({ "profile": "projprof" }));
        assert_eq!(
            active_profile(&flag, &env, &project).as_deref(),
            Some("flagprof"),
            "the flag profile wins"
        );
        assert_eq!(
            active_profile(&ConfigLayer::new(), &env, &project).as_deref(),
            Some("envprof"),
            "TMX_PROFILE wins over the project when no flag"
        );
        assert_eq!(
            active_profile(&ConfigLayer::new(), &ConfigLayer::new(), &project).as_deref(),
            Some("projprof"),
            "the project profile key is the fallback"
        );
        assert!(
            active_profile(
                &ConfigLayer::new(),
                &ConfigLayer::new(),
                &ConfigLayer::new()
            )
            .is_none(),
            "no profile anywhere resolves to none"
        );
    }

    #[test]
    fn file_layer_reads_a_project_config_and_tolerates_a_missing_one() {
        let dir = std::env::temp_dir().join(format!("tmx-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        // No config file → an empty layer.
        assert!(
            file_layer(&dir).is_empty(),
            "a directory with no tmx.config.* yields an empty layer"
        );

        // A JSON project config parses into a layer.
        std::fs::write(
            dir.join("tmx.config.json"),
            "{ \"format\": \"ndjson\", \"names\": { \"a\": \"a.yaml\" } }",
        )
        .expect("write config");
        let layer = file_layer(&dir);
        assert_eq!(
            layer.get("format").and_then(Value::as_str),
            Some("ndjson"),
            "the project config file is read into the layer"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
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
    fn retention_off_and_zero_disable_otherwise_a_window_or_the_default() {
        // `off` / `0` disable the sweep; a positive integer sets the window; garbage falls back to
        // the default (a stray env var never aborts a run).
        assert_eq!(resolve_retention_with("off"), None, "`off` disables");
        assert_eq!(
            resolve_retention_with("OFF"),
            None,
            "`off` is case-insensitive"
        );
        assert_eq!(resolve_retention_with("0"), None, "`0` disables");
        assert_eq!(
            resolve_retention_with("7"),
            Some(7),
            "a positive integer sets the window"
        );
        assert_eq!(
            resolve_retention_with("garbage"),
            Some(RUN_RETENTION_DEFAULT_DAYS),
            "an unparseable token falls back to the default"
        );
        assert_eq!(
            resolve_retention_with("  14  "),
            Some(14),
            "surrounding whitespace is trimmed"
        );
    }

    #[test]
    fn parse_duration_ms_covers_every_suffix_and_rejects_garbage() {
        // Each duration suffix converts to the right millisecond count; a bare integer is seconds.
        assert_eq!(parse_duration_ms("500ms"), Some(500), "ms is milliseconds");
        assert_eq!(parse_duration_ms("30s"), Some(30_000), "s is seconds");
        assert_eq!(parse_duration_ms("5m"), Some(300_000), "m is minutes");
        assert_eq!(parse_duration_ms("1h"), Some(3_600_000), "h is hours");
        assert_eq!(
            parse_duration_ms("2"),
            Some(2_000),
            "a bare integer reads as seconds"
        );
        assert_eq!(
            parse_duration_ms("  10s "),
            Some(10_000),
            "surrounding whitespace trims"
        );
        assert_eq!(parse_duration_ms("0"), Some(0), "zero is a valid duration");

        // Negative space: a non-numeric / empty value is unparseable, never a silent default.
        assert_eq!(
            parse_duration_ms("soon"),
            None,
            "a non-numeric value is rejected"
        );
        assert_eq!(parse_duration_ms(""), None, "an empty value is rejected");
        assert_eq!(
            parse_duration_ms("ms"),
            None,
            "a bare unit with no number is rejected"
        );
    }

    #[test]
    fn grace_resolves_the_flag_then_the_default_and_honours_zero() {
        // A parseable `--grace` wins; `--grace 0` is an immediate hard stop (kept, not defaulted);
        // absent or garbage falls back to the CANCEL_GRACE_MS default.
        assert_eq!(resolve_grace_ms(Some("2s")), 2_000, "a --grace value wins");
        assert_eq!(
            resolve_grace_ms(Some("0")),
            0,
            "--grace 0 forces an immediate hard stop, not the default"
        );
        assert_eq!(
            resolve_grace_ms(None),
            CANCEL_GRACE_MS,
            "an absent --grace falls back to the CANCEL_GRACE_MS default"
        );
        assert_eq!(
            resolve_grace_ms(Some("nonsense")),
            CANCEL_GRACE_MS,
            "an unparseable --grace falls back to the default rather than aborting the run"
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
