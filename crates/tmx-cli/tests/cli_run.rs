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

// Only meaningful when `chat` is off: with the opt-in `chat` feature the ChatModel port is wired to
// the real `ChatCompletionsModel`, so a `chat-completion` Flow clears the capability check and this
// exit-5 path no longer applies (a run with no endpoint configured is then a `chat_no_endpoint`
// RunFailure → exit 1, exercised by the chat adapter's own tests). The default build keeps chat a
// denying stub, so this is the capability-check gate under the standard `cargo nextest run`.
#[cfg(not(feature = "chat"))]
#[test]
fn a_flow_needing_an_unwired_port_exits_five() {
    // O2/composition: a `chat-completion` Flow needs the ChatModel port, which is a denying stub in
    // the default build. The capability check reports it up front as an environment error → exit 5,
    // before any task runs. `chat` is used deliberately because it is a port that stays unwired unless
    // its opt-in Cargo feature is enabled.
    let dir = temp_dir("cap");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: demo\ntasks:\n  - name: check\n    type: chat-completion\n    with:\n      model: demo-model\n      messages:\n        - role: user\n          content: hi\n",
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

#[test]
fn a_file_secret_echoed_by_a_task_never_reaches_stdout() {
    // O1: a `file`-sourced secret resolves for a task that lists it, and the echoed value is redacted
    // out of both the final state on stdout and the progress on stderr — the raw value appears nowhere.
    let dir = temp_dir("file-secret");
    let secret_path = write(&dir, "token.secret", "filesecretvalue\n");
    let flow = write(
        &dir,
        "flow.yaml",
        &format!(
            "name: demo\ncontext:\n  secrets:\n    TOKEN:\n      file: {}\ntasks:\n  - name: leak\n    type: exec\n    secrets: [TOKEN]\n    with:\n      command: \"printf %s '${{{{ secrets.TOKEN }}}}'\"\n",
            secret_path.display()
        ),
    );
    let out = run_flow(&flow, &[]);

    assert_eq!(
        out.code,
        Some(0),
        "the file-secret flow runs; stderr: {}",
        out.stderr
    );
    assert!(
        !out.stdout.contains("filesecretvalue"),
        "the raw file secret must not appear on stdout, got {:?}",
        out.stdout
    );
    assert!(
        !out.stderr.contains("filesecretvalue"),
        "the raw file secret must not appear on stderr either"
    );
    let state: Value =
        serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    // The task echoed the trailing-newline-stripped file value; it is redacted in the final state.
    assert_eq!(
        state["leak"]["message"], "[REDACTED]",
        "the echoed file secret is redacted in the final state"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_file_secret_is_a_typed_resolution_error_not_a_panic() {
    // Negative space / regression: a `file` secret whose content is only a trailing newline strips to
    // "" — an empty resolved value the Masker cannot register. A task that lists it must fail with a
    // typed resolution error (surfacing as exit 1, asserted below), NEVER a Rust panic (exit 101)
    // from the downstream masker.
    let dir = temp_dir("empty-file-secret");
    let secret_path = write(&dir, "empty.secret", "\n");
    let flow = write(
        &dir,
        "flow.yaml",
        &format!(
            "name: demo\ncontext:\n  secrets:\n    TOKEN:\n      file: {}\ntasks:\n  - name: use\n    type: exec\n    secrets: [TOKEN]\n    with:\n      command: printf done\n",
            secret_path.display()
        ),
    );
    let out = run_flow(&flow, &[]);

    // A requested secret that fails to resolve surfaces as a task failure (exit 1) — the same contract
    // as any other resolution failure for a requested secret. The regression it guards is exit 101: the
    // masker used to panic on the empty resolved value before this typed error stopped the run cleanly.
    assert_eq!(
        out.code,
        Some(1),
        "an empty file secret fails the run cleanly (exit 1), not a panic (101); stderr: {}",
        out.stderr
    );
    assert_ne!(
        out.code,
        Some(101),
        "the process must not panic on an empty resolved secret"
    );
    assert!(
        out.stderr.contains("empty value"),
        "the diagnostic names the empty-secret failure, got {:?}",
        out.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_literal_secret_is_a_typed_resolution_error_not_a_panic() {
    // Negative space / regression: a `literal` secret whose value is the empty string bypasses the
    // adapter (env/file guards do not run for a literal), so it is the runner seam that must reject it.
    // A task that lists it must fail with a typed resolution error (exit 1), NEVER a Rust panic (exit
    // 101) from `masker.assert_ready` over an unregistered empty secret.
    let dir = temp_dir("empty-literal-secret");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: demo\ncontext:\n  secrets:\n    TOKEN: \"\"\ntasks:\n  - name: use\n    type: exec\n    secrets: [TOKEN]\n    with:\n      command: printf done\n",
    );
    let out = run_flow(&flow, &[]);

    assert_eq!(
        out.code,
        Some(1),
        "an empty literal secret fails the run cleanly (exit 1), not a panic (101); stderr: {}",
        out.stderr
    );
    assert_ne!(
        out.code,
        Some(101),
        "the process must not panic on an empty resolved literal secret"
    );
    assert!(
        out.stderr.contains("empty value"),
        "the diagnostic names the empty-secret failure, got {:?}",
        out.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unrequested_secret_is_never_resolved_or_bound() {
    // O2 (negative space): the context defines a secret whose `file` source points at a path that does
    // NOT exist. No task lists it, so it is never resolved — a resolution error would surface if it
    // were. The one task that runs lists nothing and succeeds, proving an unrequested secret has no
    // resolution path into scope.
    let dir = temp_dir("unrequested-secret");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: demo\ncontext:\n  secrets:\n    UNUSED:\n      file: /no/such/tmx/unrequested/secret\ntasks:\n  - name: work\n    type: exec\n    with:\n      command: printf done\n",
    );
    let out = run_flow(&flow, &[]);

    assert_eq!(
        out.code,
        Some(0),
        "the flow runs cleanly; the unrequested secret's bad source is never touched. stderr: {}",
        out.stderr
    );
    let state: Value =
        serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    assert_eq!(
        state["work"]["message"], "done",
        "the task that requested no secret ran to completion"
    );
    assert!(
        state.get("UNUSED").is_none(),
        "the unrequested secret is never bound into scope, got {state:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
