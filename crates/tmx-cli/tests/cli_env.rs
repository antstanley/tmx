// This whole crate is test code: an `expect`/`unwrap` here IS the assertion and its panic IS the
// failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items, not an
// integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests for the environment-provider lifecycle (task 25 definition of done).
//!
//! These drive the *real* compiled `tmx` binary over temp-dir fixtures, exercising the whole
//! composition — provider-manifest loading, options validation, the `BinaryProvider` and
//! `FlowProvider` adapters, the `tmx env` method mapping, and the `tmx run` ephemeral wrapper — as a
//! reviewer running it from the shell would. They are the O1/O2/O4 obligations made executable:
//!
//! - `tmx env deploy` drives a flow-provider method (run as a Flow) and a binary-provider method
//!   (a subcommand invocation), each exit 0 with a JSON summary on stdout;
//! - a forced method failure is exit 5 (`environment`), distinct from a pipeline failure (exit 1);
//! - `tmx run` provisions and cleans a standing environment per `--keep`/`--no-deploy`/`--local`,
//!   with teardown still running after a failed run;
//! - an `environment.options` block violating the manifest `optionsSchema` is rejected before any
//!   method runs (exit 3).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A unique temp directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tmx-cli-env-{tag}-{}", std::process::id()));
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

/// Run `tmx <args...>` from the fixture directory, capturing stdout/stderr and the exit code.
fn run_tmx(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmx"));
    command.current_dir(dir).args(args);
    let output = command.output().expect("the tmx binary runs");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A flow-provider manifest whose four methods each append a one-letter marker to `log_path`
/// (`bootstrap`→B, `deploy`→D, `clean`→C, `destroy`→X), so a test can observe exactly which methods
/// ran and in what order.
fn flow_provider_manifest(log_path: &Path) -> String {
    let log = log_path.display();
    format!(
        "kind: provider\nname: local-flow\ntype: flow\nmethods:\n  \
         bootstrap:\n    - {{ name: b, type: exec, with: {{ command: \"printf B >> {log}\" }} }}\n  \
         deploy:\n    - {{ name: d, type: exec, with: {{ command: \"printf D >> {log}\" }} }}\n  \
         clean:\n    - {{ name: c, type: exec, with: {{ command: \"printf C >> {log}\" }} }}\n  \
         destroy:\n    - {{ name: x, type: exec, with: {{ command: \"printf X >> {log}\" }} }}\n"
    )
}

/// Read the marker log, treating an absent file as the empty string.
fn read_log(log_path: &Path) -> String {
    std::fs::read_to_string(log_path).unwrap_or_default()
}

#[test]
fn tmx_env_deploy_drives_a_flow_provider_method_as_a_flow() {
    // O1: `tmx env deploy` maps to the provider's deploy method; a flow provider runs the method's
    // inline tasks through the same PipelineRunner. Exit 0, and the JSON summary names the provider.
    let dir = temp_dir("flow-deploy");
    let log = dir.join("markers.log");
    write(&dir, "local.provider.yaml", &flow_provider_manifest(&log));
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  name: dev\n  provider: ./local.provider.yaml\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );

    let out = run_tmx(&dir, &["env", "deploy", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(0),
        "flow-provider deploy exits 0; stderr: {}",
        out.stderr
    );
    let summary: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout must be one JSON object, got {:?}: {e}", out.stdout));
    assert_eq!(
        summary["provider"], "local-flow",
        "the summary names the provider"
    );
    assert_eq!(
        summary["method"], "deploy",
        "the summary names the method run"
    );
    assert!(
        summary["results"].as_array().is_some_and(|r| !r.is_empty()),
        "the summary carries the method's result, got {summary:?}"
    );
    // The deploy method's inline task actually ran (its marker was written), proving it ran as a Flow.
    assert_eq!(read_log(&log), "D", "exactly the deploy method's task ran");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tmx_env_up_and_down_aggregate_the_lifecycle_methods() {
    // O1: `up` = bootstrap → deploy, `down` = clean → destroy. Two invocations drive all four methods
    // in lifecycle order, observable in the marker log.
    let dir = temp_dir("aggregate");
    let log = dir.join("markers.log");
    write(&dir, "local.provider.yaml", &flow_provider_manifest(&log));
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  provider: ./local.provider.yaml\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );

    let up = run_tmx(&dir, &["env", "up", "flow.yaml"]);
    assert_eq!(up.code, Some(0), "env up exits 0; stderr: {}", up.stderr);
    assert_eq!(
        read_log(&log),
        "BD",
        "up runs bootstrap then deploy, in order"
    );

    let down = run_tmx(&dir, &["env", "down", "flow.yaml"]);
    assert_eq!(
        down.code,
        Some(0),
        "env down exits 0; stderr: {}",
        down.stderr
    );
    assert_eq!(
        read_log(&log),
        "BDCX",
        "down runs clean then destroy, in order"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_forced_flow_provider_method_failure_is_exit_five_not_a_run_failure() {
    // O2/O4: a flow-provider method whose task fails is an environment error (exit 5), distinct from a
    // pipeline RunFailure (exit 1). The deploy method aborts on a non-zero exec.
    let dir = temp_dir("flow-fail");
    write(
        &dir,
        "local.provider.yaml",
        "kind: provider\nname: local-flow\ntype: flow\nmethods:\n  bootstrap: \"noop\"\n  deploy:\n    - { name: d, type: exec, with: { command: \"exit 1\" } }\n  clean:\n    - { name: c, type: exec, with: { command: \"printf ok\" } }\n  destroy: \"noop\"\n",
    );
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  provider: ./local.provider.yaml\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );

    let out = run_tmx(&dir, &["env", "deploy", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(5),
        "a failed flow-provider method is exit 5 (environment), not exit 1; stderr: {}",
        out.stderr
    );
    assert_ne!(
        out.code,
        Some(1),
        "a provider-method failure must not read as a pipeline RunFailure"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn out_of_schema_options_are_rejected_before_any_method_runs() {
    // O2 (negative space): an `environment.options` block violating the manifest's `optionsSchema` is
    // rejected at preflight — exit 3 — before the deploy method runs (its marker is never written).
    let dir = temp_dir("bad-options");
    let log = dir.join("markers.log");
    write(
        &dir,
        "local.provider.yaml",
        &format!(
            "kind: provider\nname: local-flow\ntype: flow\noptionsSchema:\n  type: object\n  required: [cluster]\nmethods:\n  bootstrap: \"noop\"\n  deploy:\n    - {{ name: d, type: exec, with: {{ command: \"printf D >> {}\" }} }}\n  clean: \"noop\"\n  destroy: \"noop\"\n",
            log.display()
        ),
    );
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  provider: ./local.provider.yaml\n  options:\n    region: eu\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );

    let out = run_tmx(&dir, &["env", "deploy", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(3),
        "an out-of-schema options block is a validation error (exit 3); stderr: {}",
        out.stderr
    );
    assert!(
        read_log(&log).is_empty(),
        "the deploy method must not run when options are rejected, log was {:?}",
        read_log(&log)
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tmx_run_deploys_runs_and_cleans_by_default_and_honours_the_flags() {
    // O1: `tmx run` wraps the pipeline in the ephemeral lifecycle. Default = deploy → run → clean;
    // --keep = deploy → run; --no-deploy = run only; --local = run only (no provider). The marker log
    // records exactly which provider methods ran.
    for (flag, expected_log) in [
        (None, "DC"),
        (Some("--keep"), "D"),
        (Some("--no-deploy"), ""),
        (Some("--local"), ""),
    ] {
        let tag = flag.unwrap_or("default").trim_start_matches('-');
        let dir = temp_dir(&format!("run-{tag}"));
        let log = dir.join("markers.log");
        write(&dir, "local.provider.yaml", &flow_provider_manifest(&log));
        write(
            &dir,
            "flow.yaml",
            "name: demo\nenvironment:\n  provider: ./local.provider.yaml\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
        );

        let mut args = vec!["run", "flow.yaml"];
        if let Some(flag) = flag {
            args.push(flag);
        }
        let out = run_tmx(&dir, &args);
        assert_eq!(
            out.code,
            Some(0),
            "the wrapped run exits 0 for {flag:?}; stderr: {}",
            out.stderr
        );
        // stdout is still the main flow's final state, unpolluted by provider runs.
        let state: Value = serde_json::from_str(out.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is the main run's JSON state for {flag:?}: {e}"));
        assert_eq!(
            state["work"]["message"], "worked",
            "the main task ran and merged"
        );
        assert_eq!(
            read_log(&log),
            expected_log,
            "provider lifecycle for {flag:?} must be exactly {expected_log:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn tmx_run_teardown_still_runs_after_a_failed_run() {
    // O2: `clean` runs best-effort even after a failed run. The main flow fails a task (exit 1), but
    // the marker log still records deploy AND clean (DC).
    let dir = temp_dir("run-fail-teardown");
    let log = dir.join("markers.log");
    write(&dir, "local.provider.yaml", &flow_provider_manifest(&log));
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  provider: ./local.provider.yaml\ntasks:\n  - name: gate\n    type: assert\n    with:\n      assertions:\n        - actual: 1\n          matcher: toBe\n          expected: 2\n",
    );

    let out = run_tmx(&dir, &["run", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(1),
        "the run itself fails (exit 1) on the failed assert; stderr: {}",
        out.stderr
    );
    assert_eq!(
        read_log(&log),
        "DC",
        "clean still runs after a failed run — deploy then clean both fired"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_flow_without_a_provider_is_unaffected_by_the_lifecycle() {
    // Regression check (certificate): a plain `tmx run flow.yaml` with no `environment.provider` still
    // preflights, runs through the PipelineRunner, and exits 0 — the provider wrapper is a no-op.
    let dir = temp_dir("no-provider");
    write(
        &dir,
        "flow.yaml",
        "name: demo\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );
    let out = run_tmx(&dir, &["run", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(0),
        "a provider-less run is unaffected; stderr: {}",
        out.stderr
    );
    let state: Value =
        serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    assert_eq!(state["work"]["message"], "worked", "the flow ran normally");

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------------------------
// Binary-provider tests — a manifest `binary` executable is invoked per method. Unix-only: they
// write and chmod a shell script, and the process runner spawns it via `sh -c`.
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
fn write_executable(dir: &Path, name: &str, contents: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = write(dir, name, contents);
    let mut perms = std::fs::metadata(&path).expect("stat script").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod +x the script");
    path
}

#[cfg(unix)]
const PROVIDER_SCRIPT: &str = "#!/bin/sh\ncase \"$1\" in\n  deploy) printf '{\"deployed\":true}';;\n  fail) exit 7;;\n  *) printf '{\"ok\":true}';;\nesac\n";

#[cfg(unix)]
#[test]
fn tmx_env_deploy_drives_a_binary_provider_method() {
    // O1: `tmx env deploy` against a binary provider invokes the manifest binary with the `deploy`
    // subcommand; the process result (its stdout JSON) is the method result. Exit 0.
    let dir = temp_dir("bin-deploy");
    let script = write_executable(&dir, "provider.sh", PROVIDER_SCRIPT);
    write(
        &dir,
        "bin.provider.yaml",
        &format!(
            "kind: provider\nname: bin\ntype: binary\nbinary: {}\nmethods:\n  bootstrap: bootstrap\n  deploy: deploy\n  clean: clean\n  destroy: destroy\n",
            script.display()
        ),
    );
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  name: dev\n  provider: ./bin.provider.yaml\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );

    let out = run_tmx(&dir, &["env", "deploy", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(0),
        "binary-provider deploy exits 0; stderr: {}",
        out.stderr
    );
    let summary: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is one JSON object, got {:?}: {e}", out.stdout));
    assert_eq!(
        summary["provider"], "bin",
        "the summary names the binary provider"
    );
    assert_eq!(
        summary["results"][0]["output"],
        serde_json::json!({ "deployed": true }),
        "the binary's stdout JSON is the method result"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn a_forced_binary_provider_method_failure_is_exit_five() {
    // O2/O4: a binary method whose subcommand exits non-zero is an environment error (exit 5). The
    // process runner yields a RunFailure; the BinaryProvider re-categorises it, so the CLI maps 5.
    let dir = temp_dir("bin-fail");
    let script = write_executable(&dir, "provider.sh", PROVIDER_SCRIPT);
    write(
        &dir,
        "bin.provider.yaml",
        &format!(
            "kind: provider\nname: bin\ntype: binary\nbinary: {}\nmethods:\n  bootstrap: bootstrap\n  deploy: fail\n  clean: clean\n  destroy: destroy\n",
            script.display()
        ),
    );
    write(
        &dir,
        "flow.yaml",
        "name: demo\nenvironment:\n  provider: ./bin.provider.yaml\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf worked\n",
    );

    let out = run_tmx(&dir, &["env", "deploy", "flow.yaml"]);
    assert_eq!(
        out.code,
        Some(5),
        "a failed binary-provider method is exit 5 (environment); stderr: {}",
        out.stderr
    );
    assert_ne!(
        out.code,
        Some(1),
        "a provider-method failure is not a pipeline RunFailure"
    );

    std::fs::remove_dir_all(&dir).ok();
}
