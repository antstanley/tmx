// This whole crate is test code: an `expect`/`unwrap` here IS the assertion and its panic IS the
// failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items, not an
// integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests for the resource/analysis/scaffold command surface (task 31 definition of done).
//!
//! These drive the *real* compiled `tmx` binary over temp-dir fixtures — `validate`, `inspect`,
//! `list`, `init`, `fmt`, `provider`, `context show`, `secrets list`, and `version` — as a reviewer
//! running them from the shell would. They make the certificate obligations executable:
//!
//! - O1: each command produces its documented output; `tmx fmt` round-trips a Flow across all four
//!   formats without loss; the config layers resolve highest-to-lowest (flag > env > project);
//! - O2 (negative space): `validate`/`inspect` fail-fast on a malformed artifact (exit 3),
//!   `secrets list`/`context show` never print a raw secret, and an unknown command/flag is exit 2;
//! - the scaffold `tmx init` produces both layouts and each validates.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A unique temp directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tmx-cli-cmd-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write `contents` to `dir/name` and return its path.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write fixture");
    path
}

/// The captured result of running the binary.
struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Output {
    /// Parse stdout as one JSON object, panicking (with stderr) if it is not JSON.
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout is not JSON ({e}); stdout={:?} stderr={:?}",
                self.stdout, self.stderr
            )
        })
    }
}

/// Run `tmx <args...>` from the fixture directory (with `envs` set), capturing stdout/stderr + code.
fn run_tmx_env(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmx"));
    command.current_dir(dir).args(args);
    // A clean profile/format env by default so a host var never perturbs a test.
    command.env_remove("TMX_PROFILE");
    command.env_remove("TMX_FLOW");
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("the tmx binary runs");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Run `tmx <args...>` from the fixture directory.
fn run_tmx(dir: &Path, args: &[&str]) -> Output {
    run_tmx_env(dir, &[], args)
}

/// A minimal valid single-file Flow, scalars-before-tables so it converts to TOML too.
const FLOW_YAML: &str = "name: demo\nversion: \"1\"\ninputs:\n  count:\n    type: number\n    required: true\ntasks:\n  - name: build\n    type: exec\n    with:\n      command: echo hi\n  - name: check\n    type: exec\n    with:\n      command: echo ok\n";

#[test]
fn version_reports_cli_and_spec() {
    // O1: `tmx version` emits the CLI and supported-spec versions as machine data on stdout.
    let dir = temp_dir("version");
    let out = run_tmx(&dir, &["version"]);
    assert_eq!(out.code, Some(0), "version exits 0; stderr={}", out.stderr);
    let view = out.json();
    assert!(view["cli"].is_string(), "the CLI version is reported");
    assert_eq!(
        view["spec"],
        Value::String("0.2.0".into()),
        "the supported spec version"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn validate_passes_a_good_flow_and_fails_fast_on_a_malformed_one() {
    // O1 + O2: a valid Flow validates (exit 0); an unparseable artifact fails fast (exit 3) with no
    // stdout, before any finding is printed.
    let dir = temp_dir("validate");
    write(&dir, "flow.yaml", FLOW_YAML);
    let ok = run_tmx(&dir, &["validate", "flow.yaml"]);
    assert_eq!(
        ok.code,
        Some(0),
        "a valid flow validates; stderr={}",
        ok.stderr
    );

    // Negative space: a malformed (unparseable) artifact is a fail-fast validation error → exit 3.
    write(&dir, "broken.yaml", "name: demo\ntasks: : : not valid\n");
    let bad = run_tmx(&dir, &["validate", "broken.yaml"]);
    assert_eq!(
        bad.code,
        Some(3),
        "a malformed artifact is exit 3; stderr={}",
        bad.stderr
    );
    assert!(
        bad.stdout.is_empty(),
        "no stdout on a fail-fast validation error"
    );

    // A parseable but schema-invalid artifact reports an error finding and exits 3.
    write(
        &dir,
        "environment.yaml",
        "kind: environment\nplatform: 123\n",
    );
    let invalid = run_tmx(&dir, &["validate", "environment.yaml"]);
    assert_eq!(
        invalid.code,
        Some(3),
        "a schema-invalid artifact is exit 3; stderr={}",
        invalid.stderr
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn inspect_projects_the_plan_and_fails_fast_on_a_malformed_flow() {
    // O1: `tmx inspect` projects the ordered plan and declared inputs.
    let dir = temp_dir("inspect");
    write(&dir, "flow.yaml", FLOW_YAML);
    let out = run_tmx(&dir, &["inspect", "flow.yaml"]);
    assert_eq!(out.code, Some(0), "inspect exits 0; stderr={}", out.stderr);
    let view = out.json();
    assert_eq!(
        view["flow"],
        Value::String("demo".into()),
        "the flow name projects"
    );
    assert_eq!(
        view["tasks"][0]["name"],
        Value::String("build".into()),
        "task order preserved"
    );
    assert_eq!(
        view["tasks"][0]["type"],
        Value::String("exec".into()),
        "task type projected"
    );
    assert_eq!(
        view["inputs"]["count"]["required"],
        Value::Bool(true),
        "declared input projected"
    );

    // Negative space: a malformed flow fails fast (exit 3) before any projection.
    write(&dir, "bad.yaml", "name: demo\ntasks: : : oops\n");
    let bad = run_tmx(&dir, &["inspect", "-f", "bad.yaml"]);
    assert_eq!(
        bad.code,
        Some(3),
        "inspect fails fast on malformed; stderr={}",
        bad.stderr
    );
    assert!(bad.stdout.is_empty(), "no projection on a fail-fast error");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_enumerates_flows_tasks_and_inputs() {
    // O1: `tmx list` discovers flows in the cwd, and a Flow's tasks and inputs.
    let dir = temp_dir("list");
    write(&dir, "flow.yaml", FLOW_YAML);
    let flows = run_tmx(&dir, &["list", "flows"]);
    assert_eq!(
        flows.code,
        Some(0),
        "list flows exits 0; stderr={}",
        flows.stderr
    );
    let names: Vec<String> = flows.json()["flows"]
        .as_array()
        .expect("flows array")
        .iter()
        .map(|f| f["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "flow.yaml"),
        "the cwd flow is discovered, got {names:?}"
    );

    let tasks = run_tmx(&dir, &["list", "tasks", "flow.yaml"]);
    assert_eq!(
        tasks.json()["tasks"].as_array().map(Vec::len),
        Some(2),
        "both tasks listed"
    );
    let inputs = run_tmx(&dir, &["list", "inputs", "flow.yaml"]);
    assert_eq!(
        inputs.json()["inputs"][0]["name"],
        Value::String("count".into()),
        "the input is listed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn init_scaffolds_single_file_and_folder_layouts_that_validate() {
    // O1 + residue: both scaffold layouts are produced and each validates via `tmx validate`.
    let dir = temp_dir("init");
    let single = run_tmx(&dir, &["init", "starter"]);
    assert_eq!(
        single.code,
        Some(0),
        "init exits 0; stderr={}",
        single.stderr
    );
    assert!(
        dir.join("starter.yaml").is_file(),
        "the single-file scaffold was written"
    );
    let v = run_tmx(&dir, &["validate", "starter.yaml"]);
    assert_eq!(
        v.code,
        Some(0),
        "the single-file scaffold validates; stderr={}",
        v.stderr
    );

    let folder = run_tmx(&dir, &["init", "proj", "--folder"]);
    assert_eq!(
        folder.code,
        Some(0),
        "init --folder exits 0; stderr={}",
        folder.stderr
    );
    assert!(
        dir.join("proj/environment.yaml").is_file(),
        "the folder env was written"
    );
    assert!(
        dir.join("proj/01-greet.yaml").is_file(),
        "the folder task was written"
    );
    let ve = run_tmx(&dir, &["validate", "proj/environment.yaml"]);
    assert_eq!(
        ve.code,
        Some(0),
        "the folder environment validates; stderr={}",
        ve.stderr
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn fmt_round_trips_a_flow_across_all_four_formats_without_loss() {
    // O1: convert the Flow to each format and confirm the reloaded model is identical (proved by
    // converting each form back to JSON and comparing the JSON models).
    let dir = temp_dir("fmt");
    write(&dir, "flow.yaml", FLOW_YAML);

    let baseline = run_tmx(&dir, &["fmt", "flow.yaml", "--to", "json"]);
    assert_eq!(
        baseline.code,
        Some(0),
        "fmt to json exits 0; stderr={}",
        baseline.stderr
    );
    let model_a: Value =
        serde_json::from_str(&baseline.stdout).expect("the json conversion parses");

    for (ext, to) in [("toml", "toml"), ("jsonc", "jsonc"), ("yaml", "yaml")] {
        let converted = run_tmx(&dir, &["fmt", "flow.yaml", "--to", to]);
        assert_eq!(
            converted.code,
            Some(0),
            "fmt to {to} exits 0; stderr={}",
            converted.stderr
        );
        let name = format!("round.{ext}");
        write(&dir, &name, &converted.stdout);
        // Convert the round-tripped form back to JSON and compare the models — loss-free iff equal.
        let back = run_tmx(&dir, &["fmt", &name, "--to", "json"]);
        assert_eq!(
            back.code,
            Some(0),
            "fmt {name} to json exits 0; stderr={}",
            back.stderr
        );
        let model_b: Value =
            serde_json::from_str(&back.stdout).expect("the round-trip json parses");
        assert_eq!(
            model_b, model_a,
            "a {to} round-trip preserves the model exactly"
        );
    }

    // `--write` converts on disk, swapping the extension.
    let written = run_tmx(&dir, &["fmt", "flow.yaml", "--to", "json", "--write"]);
    assert_eq!(
        written.code,
        Some(0),
        "fmt --write exits 0; stderr={}",
        written.stderr
    );
    assert!(
        dir.join("flow.json").is_file(),
        "--write produced flow.json"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn provider_validates_registers_lists_and_rejects_a_malformed_manifest() {
    // O1 + residue: a good manifest validates and registers, `list` shows it, and a malformed
    // manifest is rejected (exit 3) and never enters the registry.
    let dir = temp_dir("provider");
    let good = "kind: provider\nname: local\ntype: flow\nmethods:\n  bootstrap: \"noop\"\n  deploy:\n    - { name: d, type: exec, with: { command: \"printf D\" } }\n  clean: \"noop\"\n  destroy: \"noop\"\n";
    write(&dir, "local.provider.yaml", good);

    let validate = run_tmx(&dir, &["provider", "validate", "local.provider.yaml"]);
    assert_eq!(
        validate.code,
        Some(0),
        "a good manifest validates; stderr={}",
        validate.stderr
    );
    assert_eq!(
        validate.json()["name"],
        Value::String("local".into()),
        "the manifest name is reported"
    );

    let register = run_tmx(&dir, &["provider", "register", "local.provider.yaml"]);
    assert_eq!(
        register.code,
        Some(0),
        "register exits 0; stderr={}",
        register.stderr
    );
    let listed = run_tmx(&dir, &["provider", "list"]);
    let names: Vec<String> = listed.json()["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .map(|p| p["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "local"),
        "the registered provider is listed, got {names:?}"
    );

    // Negative space: a manifest missing required fields is rejected (exit 3), not registered.
    write(&dir, "bad.provider.yaml", "kind: provider\nname: broken\n");
    let bad = run_tmx(&dir, &["provider", "register", "bad.provider.yaml"]);
    assert_eq!(
        bad.code,
        Some(3),
        "a malformed manifest is exit 3; stderr={}",
        bad.stderr
    );
    let after = run_tmx(&dir, &["provider", "list"]);
    let after_names: Vec<String> = after.json()["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .map(|p| p["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        !after_names.iter().any(|n| n == "broken"),
        "a rejected manifest never registers"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn secrets_and_context_show_masked_values_only() {
    // O2 negative space: neither `secrets list` nor `context show` prints a raw secret value.
    let dir = temp_dir("secrets");
    let raw = "super-secret-token-value";
    let flow = format!(
        "name: demo\ncontext:\n  env:\n    REGION: eu\n  secrets:\n    TOKEN: {raw}\n    DB:\n      env: DATABASE_URL\ntasks:\n  - name: build\n    type: exec\n    with:\n      command: echo hi\n"
    );
    write(&dir, "flow.yaml", &flow);

    let secrets = run_tmx(&dir, &["secrets", "list", "flow.yaml"]);
    assert_eq!(
        secrets.code,
        Some(0),
        "secrets list exits 0; stderr={}",
        secrets.stderr
    );
    assert!(
        !secrets.stdout.contains(raw),
        "secrets list must not print the raw value"
    );
    let needed = &secrets.json()["secretsNeeded"];
    assert!(
        needed.as_array().map(Vec::len).unwrap_or(0) >= 2,
        "both secrets are listed"
    );

    let context = run_tmx(&dir, &["context", "show", "flow.yaml"]);
    assert_eq!(
        context.code,
        Some(0),
        "context show exits 0; stderr={}",
        context.stderr
    );
    assert!(
        !context.stdout.contains(raw),
        "context show must not print the raw value"
    );
    let view = context.json();
    assert_eq!(
        view["context"]["env"]["REGION"],
        Value::String("eu".into()),
        "env vars are shown"
    );
    assert_eq!(
        view["context"]["secrets"]["TOKEN"]["value"],
        Value::String("[REDACTED]".into()),
        "the literal secret is masked"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unknown_command_or_flag_is_an_exit_2_usage_error() {
    // O2: an unknown command and an unknown flag are both CLI-local usage errors (exit 2), distinct
    // from any core category.
    let dir = temp_dir("usage");
    write(&dir, "flow.yaml", FLOW_YAML);
    let unknown_cmd = run_tmx(&dir, &["teleport"]);
    assert_eq!(unknown_cmd.code, Some(2), "an unknown command is exit 2");
    let unknown_flag = run_tmx(&dir, &["inspect", "flow.yaml", "--nope"]);
    assert_eq!(unknown_flag.code, Some(2), "an unknown flag is exit 2");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_layers_resolve_flag_over_env_over_project() {
    // O1 config precedence, observed through `tmx list flows`: the project `tmx.config.*` sets a base
    // registered name; a profile's `names` replace it; the active profile resolves flag > env >
    // project — so `--profile` (flag) beats `TMX_PROFILE` (env) beats the base project config.
    let dir = temp_dir("config");
    write(&dir, "flow.yaml", FLOW_YAML);
    write(
        &dir,
        "tmx.config.json",
        "{ \"names\": { \"base\": \"base.yaml\" }, \"profiles\": { \"ci\": { \"names\": { \"ciflow\": \"ci.yaml\" } }, \"local\": { \"names\": { \"localflow\": \"local.yaml\" } } } }",
    );

    let registered = |out: &Output| -> Vec<String> {
        out.json()["flows"]
            .as_array()
            .expect("flows array")
            .iter()
            .filter(|f| f["registered"] == Value::Bool(true))
            .map(|f| f["name"].as_str().unwrap_or_default().to_string())
            .collect()
    };

    // No profile: the base project names resolve.
    let base = run_tmx(&dir, &["list", "flows"]);
    assert_eq!(
        registered(&base),
        vec!["base".to_string()],
        "the project base names resolve"
    );

    // TMX_PROFILE=ci (env) selects the ci profile's names over the base.
    let env_ci = run_tmx_env(&dir, &[("TMX_PROFILE", "ci")], &["list", "flows"]);
    assert_eq!(
        registered(&env_ci),
        vec!["ciflow".to_string()],
        "the env profile overrides the base"
    );

    // --profile local (flag) beats TMX_PROFILE=ci (env): the flag's profile wins.
    let flag_local = run_tmx_env(
        &dir,
        &[("TMX_PROFILE", "ci")],
        &["--profile", "local", "list", "flows"],
    );
    assert_eq!(
        registered(&flag_local),
        vec!["localflow".to_string()],
        "the flag profile beats the env profile"
    );
    std::fs::remove_dir_all(&dir).ok();
}
