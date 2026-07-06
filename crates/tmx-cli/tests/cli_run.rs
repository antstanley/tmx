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
    run_flow_args(flow, &[], env)
}

/// Run `tmx run <flow> <extra args…>` with the given env vars, capturing stdout/stderr and the exit
/// code. The extra args carry the reporter flags (`--format`, `--color`, …) the format tests exercise.
fn run_flow_args(flow: &Path, extra: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmx"));
    // These e2e tests exercise the reporter/exit-code path, not the run store; `--no-store` keeps every
    // run from writing a `./.tmx/runs/<id>/` dir into the crate source tree (the process cwd here is the
    // package root, not a temp fixture) while leaving the observed stdout/stderr/exit behaviour unchanged.
    command.arg("run").arg(flow).arg("--no-store");
    for arg in extra {
        command.arg(arg);
    }
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

#[test]
fn format_ndjson_streams_the_ordered_event_set_to_stdout() {
    // O1/O4: `--format ndjson` puts one JSON Event per line on stdout — the ordered canonical stream,
    // run.start first and run.finish last — while the human progress stays on stderr. stdout is the
    // event stream, NOT the final-state object.
    let dir = temp_dir("fmt-ndjson");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let out = run_flow_args(&flow, &["--format", "ndjson"], &[]);

    assert_eq!(
        out.code,
        Some(0),
        "the ndjson run exits 0; stderr: {}",
        out.stderr
    );

    // Every stdout line is a JSON object internally tagged on `event`.
    let tags: Vec<String> = out
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("each ndjson line is JSON, got {line:?}: {e}"));
            value["event"]
                .as_str()
                .unwrap_or_else(|| panic!("each event carries an `event` tag: {line}"))
                .to_string()
        })
        .collect();

    assert_eq!(
        tags.first().map(String::as_str),
        Some("run.start"),
        "run.start opens the stream"
    );
    assert_eq!(
        tags.last().map(String::as_str),
        Some("run.finish"),
        "run.finish closes the stream"
    );
    assert!(
        tags.iter().any(|t| t == "task.finish"),
        "task.finish events are streamed, got {tags:?}"
    );
    // Ordering: the build task finishes before the check task starts (the sequential loop order).
    let build_finish = out
        .stdout
        .lines()
        .position(|l| l.contains("\"event\":\"task.finish\"") && l.contains("\"build\""));
    let check_start = out
        .stdout
        .lines()
        .position(|l| l.contains("\"event\":\"task.start\"") && l.contains("\"check\""));
    assert!(
        matches!((build_finish, check_start), (Some(b), Some(c)) if b < c),
        "events stream in execution order (build.finish before check.start), got {tags:?}"
    );

    // stdout is the event stream, not the final-state object; progress is still on stderr.
    assert!(
        serde_json::from_str::<Value>(out.stdout.trim()).is_err(),
        "ndjson stdout is a stream of lines, not one final-state object"
    );
    assert!(
        out.stderr.contains("run:") && out.stderr.contains("task:"),
        "progress on stderr"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn format_pretty_writes_nothing_to_stdout_and_progress_to_stderr() {
    // O1/O4: `--format pretty` keeps the human summary on stderr and writes NOTHING to stdout — a
    // reviewer diffing the three formats sees an empty pretty stdout confined against the json object.
    let dir = temp_dir("fmt-pretty");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let out = run_flow_args(&flow, &["--format", "pretty"], &[]);

    assert_eq!(
        out.code,
        Some(0),
        "the pretty run exits 0; stderr: {}",
        out.stderr
    );
    assert!(
        out.stdout.trim().is_empty(),
        "pretty writes nothing to stdout, got {:?}",
        out.stdout
    );
    assert!(
        out.stderr.contains("run:") && out.stderr.contains("task:"),
        "the human summary is on stderr, got {:?}",
        out.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn format_json_stdout_is_the_final_state_object_matching_the_pipe_default() {
    // O1/regression: `--format json` renders the final Pipeline state as one object on stdout — the
    // identical machine-data contract the pipe default (no flag) already yields.
    let dir = temp_dir("fmt-json");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let explicit = run_flow_args(&flow, &["--format", "json"], &[]);
    let default = run_flow(&flow, &[]);

    assert_eq!(
        explicit.code,
        Some(0),
        "the json run exits 0; stderr: {}",
        explicit.stderr
    );
    let a: Value = serde_json::from_str(explicit.stdout.trim())
        .unwrap_or_else(|e| panic!("json stdout is one object: {e}"));
    let b: Value = serde_json::from_str(default.stdout.trim())
        .unwrap_or_else(|e| panic!("the pipe default stdout is one object: {e}"));
    assert_eq!(
        a, b,
        "explicit --format json equals the pipe default final-state object"
    );
    assert_eq!(
        a["build"]["message"], "built-ok",
        "the final state carries the merged output"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_secret_is_redacted_under_every_format() {
    // O2/O4: a task echoes a requested secret; the raw value appears in NO stream under any format —
    // the ndjson event stream, the json final state, and the pretty stderr summary are all masked.
    const SECRET_YAML: &str = "name: demo\ncontext:\n  secrets:\n    TOKEN:\n      env: TMX_TEST_TOKEN\ntasks:\n  - name: leak\n    type: exec\n    secrets: [TOKEN]\n    with:\n      command: \"printf %s '${{ secrets.TOKEN }}'\"\n";
    let dir = temp_dir("fmt-secret");
    let flow = write(&dir, "flow.yaml", SECRET_YAML);
    let raw = "supersecretvalueacrossformats";

    for format in ["pretty", "json", "ndjson"] {
        let out = run_flow_args(&flow, &["--format", format], &[("TMX_TEST_TOKEN", raw)]);
        assert_eq!(
            out.code,
            Some(0),
            "the {format} secret run exits 0; stderr: {}",
            out.stderr
        );
        assert!(
            !out.stdout.contains(raw),
            "the raw secret must not appear on stdout under --format {format}, got {:?}",
            out.stdout
        );
        assert!(
            !out.stderr.contains(raw),
            "the raw secret must not appear on stderr under --format {format}, got {:?}",
            out.stderr
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

// =====================================================================================
// Task 30 — the full `tmx run` flag surface, driven end to end through the real binary.
// =====================================================================================

/// Run the binary with arbitrary `args` from working directory `cwd`, capturing the result.
fn run_binary(cwd: &Path, args: &[&str]) -> Output {
    run_binary_env(cwd, args, &[])
}

/// Run the binary with arbitrary `args` and extra `env` vars from working directory `cwd`. The
/// task-34 config/env tests drive the layered-config precedence (`TMX_CONCURRENCY`, `TMX_NO_ENV`,
/// `TMX_INPUT_<NAME>`, `--profile`) through the real binary, so each var is set on the child process.
fn run_binary_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmx"));
    command.current_dir(cwd).args(args);
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

const DRY_RUN_YAML: &str = r#"name: sidefx
tasks:
  - name: touch
    type: file
    with:
      operation: write
      path: SENTINEL_PATH
      content: made
"#;

#[test]
fn dry_run_prints_the_plan_and_executes_no_task() {
    // O1: `--dry-run` resolves + validates + prints the plan and runs nothing (no file-write side
    // effect); the same flow run for real does write the sentinel — the difference is the point.
    let dir = temp_dir("dry-run");
    let sentinel = dir.join("sentinel.txt");
    let flow = write(
        &dir,
        "flow.yaml",
        &DRY_RUN_YAML.replace("SENTINEL_PATH", sentinel.to_str().expect("utf8 path")),
    );
    let flow_arg = flow.to_str().expect("utf8 path");

    let out = run_binary(&dir, &["run", flow_arg, "--no-store", "--dry-run"]);
    assert_eq!(out.code, Some(0), "dry-run exits 0; stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("\"dryRun\": true"),
        "the plan prints to stdout, got {:?}",
        out.stdout
    );
    assert!(
        out.stdout.contains("touch"),
        "the plan lists the task by name"
    );
    assert!(
        !sentinel.exists(),
        "dry-run must not run the file-write task"
    );

    let real = run_binary(&dir, &["run", flow_arg, "--no-store"]);
    assert_eq!(
        real.code,
        Some(0),
        "the real run exits 0; stderr: {}",
        real.stderr
    );
    assert!(
        sentinel.exists(),
        "the real run does run the file-write task"
    );

    std::fs::remove_dir_all(&dir).ok();
}

const MATRIX_YAML: &str = r#"name: mtx
tasks:
  - name: rec
    type: file
    with:
      operation: append
      path: OUT_PATH
      content: "${{ matrix.a }}-${{ matrix.b }}|"
"#;

#[test]
fn matrix_runs_the_full_cross_product_binding_each_axis() {
    // O1: `--matrix a=1,2 --matrix b=x,y` runs the four-way cross-product, each combination binding
    // `${{ matrix.a }}`/`${{ matrix.b }}` — observed by appending each combination to a shared file.
    let dir = temp_dir("matrix");
    let out_file = dir.join("out.txt");
    let flow = write(
        &dir,
        "flow.yaml",
        &MATRIX_YAML.replace("OUT_PATH", out_file.to_str().expect("utf8 path")),
    );
    let flow_arg = flow.to_str().expect("utf8 path");

    let out = run_binary(
        &dir,
        &[
            "run",
            flow_arg,
            "--no-store",
            "--matrix",
            "a=1,2",
            "--matrix",
            "b=x,y",
        ],
    );
    assert_eq!(
        out.code,
        Some(0),
        "the matrix run exits 0; stderr: {}",
        out.stderr
    );

    let recorded = std::fs::read_to_string(&out_file).expect("the append file exists");
    for combo in ["1-x", "1-y", "2-x", "2-y"] {
        assert!(
            recorded.contains(combo),
            "the cross-product ran combination {combo}, got {recorded:?}"
        );
    }
    assert_eq!(
        recorded.matches('|').count(),
        4,
        "exactly four combinations ran, got {recorded:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

const MATRIX_ASSERT_YAML: &str = r#"name: mtxfail
tasks:
  - name: gate
    type: assert
    with:
      assertions:
        - actual: "${{ matrix.a }}"
          matcher: toBe
          expected: 2
"#;

#[test]
fn matrix_with_a_failing_combination_exits_one_even_when_a_later_one_passes() {
    // Regression (07 §Matrix sugar, §Exit codes): a matrix lowers to a `map`, so a combination that
    // *completes* with a failed `assert` is a run failure of the whole matrix. `--matrix a=1,2` asserts
    // `${{ matrix.a }} == 2`, so a=1 fails and a=2 passes. A later passing combination must NEVER mask
    // the earlier failure: the process must exit 1, not 0.
    let dir = temp_dir("matrix-fail");
    let flow = write(&dir, "flow.yaml", MATRIX_ASSERT_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");

    let out = run_binary(&dir, &["run", flow_arg, "--no-store", "--matrix", "a=1,2"]);
    assert_eq!(
        out.code,
        Some(1),
        "a matrix with a failing combination exits 1 even when a later one passes; stderr: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("failed"),
        "the failed combination is reported on stderr, got {:?}",
        out.stderr
    );

    // The mirror case: every combination passing exits 0, so the exit-1 above is caused by the failure,
    // not by the matrix path itself.
    let ok = run_binary(&dir, &["run", flow_arg, "--no-store", "--matrix", "a=2,2"]);
    assert_eq!(
        ok.code,
        Some(0),
        "a matrix whose every combination passes exits 0; stderr: {}",
        ok.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

const SLICE_YAML: &str = r#"name: slice
tasks:
  - name: build
    type: file
    with:
      operation: write
      path: BUILD_PATH
      content: ran
  - name: verify
    type: assert
    with:
      assertions:
        - actual: "${{ tasks.build.sha }}"
          matcher: toEqual
          expected: abc
"#;

#[test]
fn from_slice_paired_with_state_in_resumes_reading_prior_state() {
    // O1: `--from verify --state-in seed.json` runs only `verify`, which reads the seeded prior
    // `build.sha` via `${{ tasks.build.sha }}`; the sliced-out `build` task never runs.
    let dir = temp_dir("slice");
    let build_sentinel = dir.join("build.txt");
    let flow = write(
        &dir,
        "flow.yaml",
        &SLICE_YAML.replace("BUILD_PATH", build_sentinel.to_str().expect("utf8 path")),
    );
    let flow_arg = flow.to_str().expect("utf8 path");
    let seed = write(&dir, "seed.json", "{\"build\":{\"sha\":\"abc\"}}");
    let seed_arg = seed.to_str().expect("utf8 path");

    let out = run_binary(
        &dir,
        &[
            "run",
            flow_arg,
            "--no-store",
            "--from",
            "verify",
            "--state-in",
            seed_arg,
        ],
    );
    assert_eq!(
        out.code,
        Some(0),
        "the resumed slice reads the seeded prior state and exits 0; stderr: {}",
        out.stderr
    );
    assert!(
        !build_sentinel.exists(),
        "the sliced-out `build` task never runs"
    );

    // Negative space: the same slice without the seed cannot read the prior state — non-zero exit.
    std::fs::remove_file(&build_sentinel).ok();
    let unseeded = run_binary(&dir, &["run", flow_arg, "--no-store", "--from", "verify"]);
    assert_ne!(
        unseeded.code,
        Some(0),
        "without --state-in the resumed slice cannot read prior state; stderr: {}",
        unseeded.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

const INPUT_YAML: &str = r#"name: inp
inputs:
  count:
    type: number
tasks:
  - name: verify
    type: assert
    with:
      assertions:
        - actual: "${{ inputs.count }}"
          matcher: toEqual
          expected: 3
"#;

#[test]
fn typed_input_is_coerced_to_its_declared_type() {
    // O1: `--input count=3` coerces the string to the declared `number`, so the assert comparing it
    // against the number `3` holds; a non-numeric value fails coercion (negative space, exit 3).
    let dir = temp_dir("input");
    let flow = write(&dir, "flow.yaml", INPUT_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");

    let out = run_binary(&dir, &["run", flow_arg, "--no-store", "--input", "count=3"]);
    assert_eq!(
        out.code,
        Some(0),
        "the coerced numeric input satisfies the assert; stderr: {}",
        out.stderr
    );

    let bad = run_binary(
        &dir,
        &["run", flow_arg, "--no-store", "--input", "count=lots"],
    );
    assert_eq!(
        bad.code,
        Some(3),
        "a value that does not coerce to the declared type is a validation error (exit 3); stderr: {}",
        bad.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn state_out_dumps_the_final_state_for_a_later_state_in() {
    // O1: `--state-out` writes the masked final state to a file, which then seeds a follow-up run's
    // `--state-in` — the round trip a resumed slice relies on.
    let dir = temp_dir("state-out");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");
    let out = dir.join("state.json");
    let out_arg = out.to_str().expect("utf8 path");

    let run = run_binary(
        &dir,
        &["run", flow_arg, "--no-store", "--state-out", out_arg],
    );
    assert_eq!(run.code, Some(0), "the run exits 0; stderr: {}", run.stderr);
    assert!(out.exists(), "--state-out wrote the final-state file");

    let dumped: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read state-out"))
            .expect("the dumped state is valid JSON");
    assert!(dumped.is_object(), "the dumped state is a JSON object");
    assert!(
        dumped.get("build").is_some(),
        "the dumped state carries the build task's output, got {dumped:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// =====================================================================================
// Task 34 — layered config + env bind `tmx run` (07 §Configuration): the `TMX_CONCURRENCY`/
// `TMX_MAX_STATE_SIZE`/project-config/`--profile` caps, `TMX_NO_ENV`, and `TMX_INPUT_<NAME>`.
// =====================================================================================

const ECHO_INPUT_YAML: &str = r#"name: inp
inputs:
  foo:
    type: string
tasks:
  - name: echo
    type: exec
    with:
      command: "printf %s '${{ inputs.foo }}'"
"#;

#[test]
fn tmx_input_env_reaches_state_and_an_explicit_input_overrides_it() {
    // O2: `TMX_INPUT_FOO=bar` supplies the declared `foo` input, reaching state as `${{ inputs.foo }}`;
    // an explicit `--input foo=baz` outranks the env value.
    let dir = temp_dir("tmx-input");
    let flow = write(&dir, "flow.yaml", ECHO_INPUT_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");

    // The env var supplies the input.
    let from_env = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store"],
        &[("TMX_INPUT_FOO", "bar")],
    );
    assert_eq!(
        from_env.code,
        Some(0),
        "the env-supplied input runs; stderr: {}",
        from_env.stderr
    );
    let state: Value = serde_json::from_str(from_env.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    assert_eq!(
        state["echo"]["message"], "bar",
        "TMX_INPUT_FOO reached state as ${{ inputs.foo }}, got {state:?}"
    );

    // An explicit --input outranks the env value.
    let overridden = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store", "--input", "foo=baz"],
        &[("TMX_INPUT_FOO", "bar")],
    );
    assert_eq!(
        overridden.code,
        Some(0),
        "the overriding run exits 0; stderr: {}",
        overridden.stderr
    );
    let state: Value = serde_json::from_str(overridden.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    assert_eq!(
        state["echo"]["message"], "baz",
        "an explicit --input outranks TMX_INPUT_FOO, got {state:?}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

const PROVIDER_LOCAL_YAML: &str = r#"name: prov
environment:
  provider: ./no-such-provider.yaml
tasks:
  - name: work
    type: exec
    with:
      command: printf done
"#;

#[test]
fn tmx_no_env_suppresses_the_provider_lifecycle_as_local_does() {
    // O2: a Flow declares an `environment.provider` that does not resolve. A bare run tries to load it
    // and fails (non-zero); `TMX_NO_ENV` (like `--local`) skips the provider entirely, so the task runs
    // and the run exits 0 — the env var is the parity of the `--no-env` flag.
    let dir = temp_dir("tmx-no-env");
    let flow = write(&dir, "flow.yaml", PROVIDER_LOCAL_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");

    // Bare run: the missing provider is loaded and fails — a non-zero exit.
    let bare = run_binary(&dir, &["run", flow_arg, "--no-store"]);
    assert_ne!(
        bare.code,
        Some(0),
        "a missing provider fails a non-local run; stderr: {}",
        bare.stderr
    );

    // TMX_NO_ENV skips the provider lifecycle, so the task runs to completion.
    let no_env = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store"],
        &[("TMX_NO_ENV", "1")],
    );
    assert_eq!(
        no_env.code,
        Some(0),
        "TMX_NO_ENV skips the provider and the task runs; stderr: {}",
        no_env.stderr
    );
    let state: Value = serde_json::from_str(no_env.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is JSON: {e}"));
    assert_eq!(
        state["work"]["message"], "done",
        "the task ran with the provider suppressed, got {state:?}"
    );

    // The explicit --local flag has the same effect — the baseline the env var mirrors.
    let local = run_binary(&dir, &["run", flow_arg, "--no-store", "--local"]);
    assert_eq!(
        local.code,
        Some(0),
        "--local also skips the provider; stderr: {}",
        local.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_malformed_numeric_env_value_is_a_usage_error_exit_two() {
    // O3 (negative space): a malformed `TMX_CONCURRENCY`/`TMX_MAX_STATE_SIZE` is a usage error (exit 2),
    // surfaced before the run — never silently ignored. A well-formed value runs normally (exit 0).
    let dir = temp_dir("tmx-bad-num");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");

    let bad_concurrency = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store"],
        &[("TMX_CONCURRENCY", "x")],
    );
    assert_eq!(
        bad_concurrency.code,
        Some(2),
        "a non-numeric TMX_CONCURRENCY is a usage error (exit 2); stderr: {}",
        bad_concurrency.stderr
    );
    assert!(
        bad_concurrency.stdout.trim().is_empty(),
        "no machine data on stdout for a usage error, got {:?}",
        bad_concurrency.stdout
    );

    let bad_state_size = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store"],
        &[("TMX_MAX_STATE_SIZE", "big")],
    );
    assert_eq!(
        bad_state_size.code,
        Some(2),
        "a non-numeric TMX_MAX_STATE_SIZE is a usage error (exit 2); stderr: {}",
        bad_state_size.stderr
    );

    // A well-formed value is honoured, not rejected — proving exit 2 is the malformed path, not blanket.
    let ok = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store"],
        &[("TMX_CONCURRENCY", "2")],
    );
    assert_eq!(
        ok.code,
        Some(0),
        "a well-formed TMX_CONCURRENCY runs normally; stderr: {}",
        ok.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn concurrency_precedence_flag_beats_env_beats_profile_beats_project() {
    // O1: a project `tmx.config.json` sets a malformed base `concurrency`, and a named profile a valid
    // one. The bare run consults the project layer and fails (exit 2). A `--concurrency` flag, a
    // `TMX_CONCURRENCY` env var, and `--profile ok` each override the bad base and run (exit 0) — the
    // documented `flag > env > profile/project` precedence, observed end to end.
    let dir = temp_dir("cfg-precedence");
    let flow = write(&dir, "flow.yaml", PASSING_YAML);
    let flow_arg = flow.to_str().expect("utf8 path");
    // The base project concurrency is malformed; the `ok` profile overrides it with a valid value.
    write(
        &dir,
        "tmx.config.json",
        r#"{ "concurrency": "lots", "profiles": { "ok": { "concurrency": "4" } } }"#,
    );

    // Bare: the project layer's malformed concurrency is a usage error (exit 2) — proving the layer is
    // consulted for the run, not just for `tmx list`.
    let bare = run_binary(&dir, &["run", flow_arg, "--no-store"]);
    assert_eq!(
        bare.code,
        Some(2),
        "the project-config concurrency binds the run (malformed → exit 2); stderr: {}",
        bare.stderr
    );

    // The flag outranks the project layer.
    let by_flag = run_binary(&dir, &["run", flow_arg, "--no-store", "--concurrency", "4"]);
    assert_eq!(
        by_flag.code,
        Some(0),
        "an explicit --concurrency outranks the project layer; stderr: {}",
        by_flag.stderr
    );

    // The env var outranks the project layer.
    let by_env = run_binary_env(
        &dir,
        &["run", flow_arg, "--no-store"],
        &[("TMX_CONCURRENCY", "4")],
    );
    assert_eq!(
        by_env.code,
        Some(0),
        "TMX_CONCURRENCY outranks the project layer; stderr: {}",
        by_env.stderr
    );

    // The selected profile's concurrency outranks the base project layer.
    let by_profile = run_binary(&dir, &["--profile", "ok", "run", flow_arg, "--no-store"]);
    assert_eq!(
        by_profile.code,
        Some(0),
        "--profile ok selects the profile's valid concurrency over the bad base; stderr: {}",
        by_profile.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------------------------
// Task 36 — reference-form context / environment run end-to-end through the real binary, not just
// green at preflight. The flow's `context` / `environment` is an external-file path; the run must
// exit 0 with the referenced values available, exactly as the inline form does.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_reference_form_context_runs_end_to_end_and_a_task_reads_its_value() {
    // Task 36 O1/O4 (Reviewable): a Flow whose `context` is an external-file reference runs to exit 0,
    // and a task reads a value the referenced context supplies (`env.GREETING`) — it does NOT
    // fail-close at exit 4 nor drop the context.
    let dir = temp_dir("refcontext");
    write(
        &dir,
        "context.yaml",
        "kind: context\nname: default\nenv:\n  GREETING: hello-from-ref\n",
    );
    let flow = write(
        &dir,
        "flow.yaml",
        "name: refctx\ncontext: ./context.yaml\ntasks:\n  - name: read\n    type: assert\n    with:\n      assertions:\n        - actual: \"${{ env.GREETING }}\"\n          matcher: toBe\n          expected: hello-from-ref\n",
    );

    let out = run_flow(&flow, &[]);
    assert_eq!(
        out.code,
        Some(0),
        "a reference-form context flow runs to exit 0 (not exit 4); stderr: {}",
        out.stderr
    );
    let state: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is one JSON object, got {:?}: {e}", out.stdout));
    assert_eq!(
        state["read"]["passed"], true,
        "the assert read the referenced context's env value and held"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_reference_form_environment_runs_end_to_end() {
    // Task 36 O2: a Flow whose `environment` is an external-file reference resolves through to
    // execution and exits 0 rather than fail-closing at the engine re-load. A provider-less `local`
    // environment needs no capability, isolating the reference-inlining path.
    let dir = temp_dir("refenv");
    write(
        &dir,
        "environment.yaml",
        "kind: environment\nname: local-env\nplatform: local\n",
    );
    let flow = write(
        &dir,
        "flow.yaml",
        "name: refenv\nenvironment: ./environment.yaml\ntasks:\n  - name: gate\n    type: assert\n    with:\n      assertions:\n        - actual: 1\n          matcher: toBe\n          expected: 1\n",
    );

    let out = run_flow(&flow, &[]);
    assert_eq!(
        out.code,
        Some(0),
        "a reference-form environment flow runs to exit 0 (not exit 4); stderr: {}",
        out.stderr
    );
    let state: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is one JSON object, got {:?}: {e}", out.stdout));
    assert_eq!(
        state["gate"]["passed"], true,
        "the run executed under the referenced environment"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_dangling_reference_form_context_exits_four() {
    // Task 36 O3 (negative space): a reference-form `context` pointing at a missing file still surfaces
    // its typed resolution error → exit 4, with nothing on stdout.
    let dir = temp_dir("refcontext-dangling");
    let flow = write(
        &dir,
        "flow.yaml",
        "name: bad\ncontext: ./does-not-exist.yaml\ntasks:\n  - name: gate\n    type: assert\n    with:\n      assertions:\n        - actual: 1\n          matcher: toBe\n          expected: 1\n",
    );

    let out = run_flow(&flow, &[]);
    assert_eq!(
        out.code,
        Some(4),
        "a dangling context reference is a resolution error → exit 4; stderr: {}",
        out.stderr
    );
    assert!(
        out.stdout.trim().is_empty(),
        "no JSON on stdout for a resolution failure, got {:?}",
        out.stdout
    );

    std::fs::remove_dir_all(&dir).ok();
}
