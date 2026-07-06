// This whole crate is test code: an `expect`/`unwrap` here IS the assertion and its panic IS the
// failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items, not an
// integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests for the `tmx run` binary (task 17 definition of done).
//!
//! These drive the *real* compiled binary (`CARGO_BIN_EXE_tmx`) over temp-dir Flow files, exercising
//! the full composition — resolution, preflight, the process/assert path, masking, the stdout/stderr
//! split, and the exit-code mapping — exactly as a reviewer running it from the shell would. They are
//! the O1/O2/O4 obligations made executable: a mixed `exec`/`assert` Flow prints one masked JSON
//! object to stdout with progress on stderr and exit 0; a failed `assert` is exit 1; an unresolved
//! Flow is exit 4; a Flow needing an unwired port is exit 5 (capability check); a requested secret a
//! task echoes never reaches stdout.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A unique temp directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tmx-cli-{tag}-{}", std::process::id()));
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

/// Run `tmx run <flow>` with the given extra env vars, capturing stdout/stderr and the exit code.
fn run_flow(flow: &Path, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmx"));
    command.arg("run").arg(flow);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("the tmx binary runs");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

const PASSING_YAML: &str = r#"name: demo
tasks:
  - name: build
    type: exec
    with:
      command: printf built-ok
  - name: check
    type: assert
    with:
      assertions:
        - actual: "${{ tasks.build.message }}"
          matcher: toBe
          expected: built-ok
"#;

#[test]
fn passing_exec_assert_flow_prints_masked_json_to_stdout_with_progress_on_stderr() {
    // O1 + O4: a mixed exec/assert Flow runs, stdout is one JSON object (the merged final state), the
    // assert reads the prior exec via `${{ tasks.* }}`, progress lands on stderr, and the exit is 0.
    let dir = temp_dir("passing");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let out = run_flow(&flow, &[]);

    assert_eq!(
        out.code,
        Some(0),
        "a passing flow exits 0; stderr: {}",
        out.stderr
    );
    let state: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout must be one JSON object, got {:?}: {e}", out.stdout));
    assert_eq!(
        state["build"]["message"], "built-ok",
        "the exec output merged under its task name"
    );
    assert_eq!(
        state["check"]["passed"], true,
        "the assert passed against the prior task's output"
    );
    // Progress is on stderr, not stdout — the `| jq` contract: stdout is JSON only.
    assert!(
        out.stderr.contains("task:") && out.stderr.contains("run:"),
        "per-task progress appears on stderr, got {:?}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("task:"),
        "no progress text leaks onto stdout"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn json_and_toml_formats_run_the_same_flow() {
    // O1: any of the four formats loads. The same logical flow in JSON and TOML both run to exit 0
    // and print the same final state — the loader lands every format in one model.
    let dir = temp_dir("formats");
    let json = write(
        &dir,
        "flow.json",
        r#"{ "name": "demo", "tasks": [ { "name": "build", "type": "exec", "with": { "command": "printf built-ok" } } ] }"#,
    );
    let toml = write(
        &dir,
        "flow.toml",
        "name = \"demo\"\n[[tasks]]\nname = \"build\"\ntype = \"exec\"\n[tasks.with]\ncommand = \"printf built-ok\"\n",
    );

    for flow in [json, toml] {
        let out = run_flow(&flow, &[]);
        assert_eq!(
            out.code,
            Some(0),
            "format run exits 0; stderr: {}",
            out.stderr
        );
        let state: Value = serde_json::from_str(out.stdout.trim())
            .unwrap_or_else(|e| panic!("stdout is JSON for {flow:?}: {e}"));
        assert_eq!(
            state["build"]["message"], "built-ok",
            "the same final state regardless of source format"
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failed_assert_exits_one_and_still_prints_state() {
    // O2: a failing assert without continueOnError aborts the run — exit 1 — while stdout still
    // carries the (partial) masked final state so `| jq` keeps working on a failure.
    let dir = temp_dir("failassert");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: demo\ntasks:\n  - name: gate\n    type: assert\n    with:\n      assertions:\n        - actual: 1\n          matcher: toBe\n          expected: 2\n",
    );
    let out = run_flow(&flow, &[]);
    assert_eq!(
        out.code,
        Some(1),
        "a failed assert exits 1; stderr: {}",
        out.stderr
    );
    assert!(
        serde_json::from_str::<Value>(out.stdout.trim()).is_ok(),
        "stdout is still valid JSON on a failed run, got {:?}",
        out.stdout
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unresolved_flow_exits_four() {
    // O2: an explicitly-named but missing Flow is a resolution error → exit 4, with the diagnostic on
    // stderr and nothing on stdout.
    let out = run_flow(Path::new("/nonexistent/tmx-does-not-exist.yaml"), &[]);
    assert_eq!(
        out.code,
        Some(4),
        "an unresolved flow exits 4; stderr: {}",
        out.stderr
    );
    assert!(
        out.stdout.trim().is_empty(),
        "no JSON on stdout for a resolution failure"
    );
    assert!(
        out.stderr.contains("tmx:"),
        "the error is reported on stderr"
    );
}

#[test]
fn a_flow_needing_an_unwired_port_exits_five() {
    // O2/composition: a `file` Flow needs the FileSystem port, which is a denying stub in this build.
    // The capability check reports it up front as an environment error → exit 5, before any task runs.
    let dir = temp_dir("cap");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: demo\ntasks:\n  - name: check\n    type: file\n    with:\n      operation: exists\n      path: /tmp/tmx-nonexistent\n",
    );
    let out = run_flow(&flow, &[]);
    assert_eq!(
        out.code,
        Some(5),
        "a missing capability exits 5; stderr: {}",
        out.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_requested_secret_echoed_by_a_task_never_reaches_stdout() {
    // O2/residue: a task requests a secret and echoes it into its output; the masker redacts it out of
    // both the final state on stdout and the progress on stderr — the raw value appears nowhere.
    let dir = temp_dir("secret");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: demo\ncontext:\n  secrets:\n    TOKEN:\n      env: TMX_TEST_TOKEN\ntasks:\n  - name: leak\n    type: exec\n    secrets: [TOKEN]\n    with:\n      command: \"printf %s '${{ secrets.TOKEN }}'\"\n",
    );
    let out = run_flow(&flow, &[("TMX_TEST_TOKEN", "supersecretvalue")]);

    assert_eq!(
        out.code,
        Some(0),
        "the secret flow runs; stderr: {}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("supersecretvalue"),
        "the raw secret must not appear on stdout, got {:?}",
        out.stdout
    );
    assert!(
        !out.stderr.contains("supersecretvalue"),
        "the raw secret must not appear on stderr either"
    );
    let state: Value =
        serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    assert_eq!(
        state["leak"]["message"], "[REDACTED]",
        "the echoed secret is redacted in the final state"
    );

    std::fs::remove_dir_all(&dir).ok();
}
