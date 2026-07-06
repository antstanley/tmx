// This whole crate is test code: an `expect`/`unwrap` here IS the assertion and its panic IS the
// failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items, not an
// integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests for `tmx lint` and `tmx run --check-produces` (Task 28 O4, reviewable).
//!
//! These drive the *real* compiled binary over temp-dir fixtures: a Flow carrying a seeded dataflow
//! defect (a typo'd `${{ tasks.build.artifcat }}` read) lints to a warning at exit 0 and, under
//! `--strict`, to an exit-3 error — a depth pure schema `validate` never reaches, since the Flow is
//! structurally valid. A separate Flow whose task output violates its `produces` schema exercises the
//! three runtime states: `--check-produces=strict` fails the run (exit 1), a bare `--check-produces`
//! warns and the run succeeds (exit 0), and an absent flag runs unchecked (exit 0).

use std::path::{Path, PathBuf};
use std::process::Command;

/// A unique temp directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tmx-cli-lint-{tag}-{}", std::process::id()));
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

/// Run `tmx <args…>` capturing stdout/stderr and the exit code.
fn run(args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_tmx"))
        .args(args)
        .output()
        .expect("the tmx binary runs");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A structurally-valid Flow that nonetheless carries a dataflow defect: `ship` reads
/// `tasks.build.artifcat`, a typo of `build`'s declared `produces` property `artifact`.
const TYPO_YAML: &str = r#"name: deploy
inputs:
  name:
    type: string
tasks:
  - name: build
    type: exec
    with:
      command: printf built
    produces:
      type: object
      properties:
        artifact:
          type: string
  - name: ship
    type: exec
    with:
      command: "echo ${{ tasks.build.artifcat }}"
"#;

#[test]
fn lint_flags_a_typo_as_a_warning_at_exit_zero_and_strict_promotes_it_to_exit_three() {
    // O1 + O4: a Flow with a typo'd produces read lints to a warning (exit 0, the finding on stderr);
    // under `--strict` the same warning becomes an exit-3 error — the validate-vs-lint depth split.
    let dir = temp_dir("typo");
    let flow = write(&dir, "flow.yaml", TYPO_YAML);

    let bare = run(&["lint", flow.to_str().expect("utf-8 path")]);
    assert_eq!(
        bare.code,
        Some(0),
        "a bare lint reports warnings but exits 0; stderr: {}",
        bare.stderr
    );
    assert!(
        bare.stderr.contains("produces_field_unknown"),
        "the typo'd produces read is reported on stderr, got: {}",
        bare.stderr
    );
    assert!(
        bare.stdout.trim().is_empty(),
        "lint writes no machine data to stdout, got: {:?}",
        bare.stdout
    );

    let strict = run(&["lint", flow.to_str().expect("utf-8 path"), "--strict"]);
    assert_eq!(
        strict.code,
        Some(3),
        "under --strict the warning is an exit-3 error; stderr: {}",
        strict.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A Flow whose single task's captured output (`{ "message": … }`) violates a `produces` schema that
/// requires a numeric `count` and forbids additional properties.
const VIOLATES_PRODUCES_YAML: &str = r#"name: build-flow
tasks:
  - name: build
    type: exec
    with:
      command: printf hi
    produces:
      type: object
      additionalProperties: false
      required: [count]
      properties:
        count:
          type: number
"#;

#[test]
fn check_produces_strict_fails_while_warn_and_absent_let_the_run_succeed() {
    // O2 + O4: the three runtime states over a task whose output violates its produces schema —
    // strict fails the run (exit 1), a bare --check-produces warns but succeeds (exit 0), and an
    // absent flag runs unchecked (exit 0).
    let dir = temp_dir("produces");
    let flow = write(&dir, "flow.yaml", VIOLATES_PRODUCES_YAML);
    let path = flow.to_str().expect("utf-8 path");

    let absent = run(&["run", path, "--no-store"]);
    assert_eq!(
        absent.code,
        Some(0),
        "without the flag the produces check never runs; stderr: {}",
        absent.stderr
    );

    let warn = run(&["run", path, "--no-store", "--check-produces"]);
    assert_eq!(
        warn.code,
        Some(0),
        "a bare --check-produces warns but the run succeeds; stderr: {}",
        warn.stderr
    );

    let strict = run(&["run", path, "--no-store", "--check-produces=strict"]);
    assert_eq!(
        strict.code,
        Some(1),
        "--check-produces=strict fails the violating task (exit 1); stderr: {}",
        strict.stderr
    );

    std::fs::remove_dir_all(&dir).ok();
}
