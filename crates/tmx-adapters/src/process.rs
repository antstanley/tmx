//! The OS-process [`ProcessRunner`] adapter — the first real side-effecting adapter.
//!
//! [`OsProcessRunner`] runs the two process-shaped task types behind the `ProcessRunner` port
//! ([`.specs/06-ports-and-adapters.md` §Executor ports](../../../../.specs/06-ports-and-adapters.md)):
//!
//! - **`exec`** ([`ProcessKind::Exec`]) — a single shell command line, run as `sh -c <command>`.
//! - **`run`** ([`ProcessKind::Run`]) — a script in a named language/interpreter, defaulting to
//!   [`DEFAULT_RUN_LANGUAGE`] (`bash`). A `command` that names an existing file is run as a script
//!   *file* (`bash <path>`); otherwise it is run as an *inline* script (`bash -c <command>`) — the
//!   two arms of "an inline `script` or a `file` path".
//!
//! Both enforce the per-task `timeout` and bound captured stdout/stderr by
//! [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES) (overridable per
//! runner for tests). A host failure — a spawn error, a non-zero exit, a timeout, or an over-cap
//! capture — is a typed [`RunError`] routed through [`From<ProcessError>`](RunError), **never** a
//! panic (06 §Adapters return typed errors, never panic on host failure). This is the crate where
//! `tokio` lives, gated behind the `process` Cargo feature so a minimal build can drop it.

use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use tmx_core::error::RunError;
use tmx_core::model::Milliseconds;
use tmx_core::ports::driven::{ProcessKind, ProcessOutput, ProcessRunner, ProcessSpec};
use tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES;

/// The default language/interpreter for a `run` task when `language` is unset — `bash`
/// (06 §exec vs run). A default *identifier*, not a numeric bound, so it lives here rather than in
/// `tmx-schema::limits` (which is reserved for numeric limits).
pub const DEFAULT_RUN_LANGUAGE: &str = "bash";

/// The size of one read chunk pulled from a child pipe, in bytes. A buffering granularity for the
/// capped reader — not an engine limit — so it is a local implementation detail, not a
/// `tmx-schema::limits` constant.
const READ_CHUNK_BYTES: usize = 8 * 1024;

/// Runs `exec` and `run` tasks as external OS processes — the built-in [`ProcessRunner`] adapter.
///
/// Holds only the captured-output cap it enforces; the default is
/// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES). Tests construct one
/// with a tiny cap via [`with_output_cap_bytes`](OsProcessRunner::with_output_cap_bytes) to exercise
/// the over-cap path without producing 64 MiB of output.
#[derive(Debug, Clone)]
pub struct OsProcessRunner {
    /// The captured-output ceiling, in bytes, applied to each of stdout and stderr.
    output_cap_bytes: u64,
}

impl Default for OsProcessRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl OsProcessRunner {
    /// A runner that bounds captured output by
    /// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
    #[must_use]
    pub fn new() -> Self {
        Self {
            output_cap_bytes: CAPTURED_OUTPUT_MAX_BYTES,
        }
    }

    /// A runner with an explicit captured-output cap, in bytes — for tests exercising the
    /// `output_too_large` path with a small, fast payload.
    #[must_use]
    pub fn with_output_cap_bytes(output_cap_bytes: u64) -> Self {
        Self { output_cap_bytes }
    }

    /// Map a [`ProcessSpec`] to the concrete `(program, leading-args)` invocation.
    ///
    /// `exec` is always `sh -c <command>`. `run` uses the task's `language` (default
    /// [`DEFAULT_RUN_LANGUAGE`]); a `command` naming an existing file is the *file* arm
    /// (`<interp> <path>`), anything else the *inline* arm (`<interp> -c <command>`).
    fn invocation(spec: &ProcessSpec) -> (String, Vec<String>) {
        match spec.kind {
            ProcessKind::Exec => (
                "sh".to_string(),
                vec!["-c".to_string(), spec.command.clone()],
            ),
            ProcessKind::Run => {
                let interpreter = spec
                    .language
                    .clone()
                    .filter(|language| !language.is_empty())
                    .unwrap_or_else(|| DEFAULT_RUN_LANGUAGE.to_string());
                if Path::new(&spec.command).is_file() {
                    (interpreter, vec![spec.command.clone()])
                } else {
                    (interpreter, vec!["-c".to_string(), spec.command.clone()])
                }
            }
        }
    }

    /// Build the [`tokio::process::Command`] for `spec`: the resolved invocation, extra `args`, the
    /// child `env` (layered over the inherited environment), and the working directory. All three
    /// stdio streams are piped so the adapter owns capture and stdin injection.
    fn build_command(spec: &ProcessSpec) -> Command {
        let (program, leading_args) = Self::invocation(spec);
        let mut command = Command::new(program);
        command.args(leading_args);
        command.args(&spec.args);
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Hard-stop on drop: when a run-level cancellation (`--timeout`/SIGINT past the grace
            // window) abandons the in-flight `run` future, dropping the child kills it rather than
            // leaving it running detached — the process side of the cancellation hard stop (task 29).
            .kill_on_drop(true);
        command
    }

    /// Run `spec` to completion, returning captured output or a typed [`ProcessError`].
    ///
    /// Stdin injection and both output captures run concurrently (via `try_join!`) so a child that
    /// interleaves reading stdin with writing a lot of stdout cannot deadlock; the whole pump is
    /// wrapped in the per-task `timeout`, and a timeout hard-kills the child before reporting.
    async fn run_inner(&self, spec: ProcessSpec) -> Result<ProcessOutput, ProcessError> {
        let mut command = Self::build_command(&spec);
        let started = Instant::now();
        let mut child = command.spawn().map_err(ProcessError::Spawn)?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProcessError::pipe("stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProcessError::pipe("stderr"))?;
        let mut stdin = child.stdin.take();
        let cap = self.output_cap_bytes;
        let stdin_data = spec.stdin.clone();

        let pump = async {
            let stdin_fut = async {
                // Always close stdin (drop the handle) so a child reading stdin sees EOF; write the
                // provided bytes first when present.
                if let Some(mut handle) = stdin.take()
                    && let Some(data) = &stdin_data
                {
                    handle.write_all(data.as_bytes()).await?;
                    handle.flush().await?;
                }
                Ok::<(), ProcessError>(())
            };
            let ((), out, err) = tokio::try_join!(
                stdin_fut,
                read_capped(&mut stdout, cap),
                read_capped(&mut stderr, cap),
            )?;
            let status = child.wait().await?;
            Ok::<(Vec<u8>, Vec<u8>, ExitStatus), ProcessError>((out, err, status))
        };

        let outcome = match spec.timeout {
            Some(Milliseconds(ms)) => {
                match tokio::time::timeout(Duration::from_millis(ms), pump).await {
                    Ok(result) => result,
                    Err(_elapsed) => {
                        // The pump future (and its borrow of `child`) is dropped here; reclaim the
                        // child to hard-stop it, then report the timeout.
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        return Err(ProcessError::Timeout { ms });
                    }
                }
            }
            None => pump.await,
        };

        let (stdout_bytes, stderr_bytes, status) = match outcome {
            Ok(triple) => triple,
            Err(error @ ProcessError::OutputTooLarge { .. }) => {
                // A capped read aborted the pump before the child exited; stop the child so it does
                // not keep running detached.
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(error);
            }
            Err(error) => return Err(error),
        };

        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if status.success() {
            Ok(ProcessOutput {
                exit_code: status.code(),
                stdout: stdout_bytes,
                stderr: stderr_bytes,
                ms: Milliseconds(ms),
            })
        } else {
            // A non-zero exit (or signal termination) is a typed RunError, not a zero-exit `Ok`.
            Err(ProcessError::NonZeroExit {
                code: status.code(),
            })
        }
    }
}

#[async_trait]
impl ProcessRunner for OsProcessRunner {
    async fn run(&self, spec: ProcessSpec) -> Result<ProcessOutput, RunError> {
        self.run_inner(spec).await.map_err(RunError::from)
    }
}

/// Read `reader` to EOF into a `Vec`, failing with [`ProcessError::OutputTooLarge`] the moment the
/// accumulated length exceeds `cap` — so a runaway child cannot grow the buffer without bound.
async fn read_capped<R>(reader: &mut R, cap: u64) -> Result<Vec<u8>, ProcessError>
where
    R: AsyncReadExt + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() as u64 > cap {
            return Err(ProcessError::OutputTooLarge { cap_bytes: cap });
        }
    }
    Ok(buffer)
}

/// The internal, adapter-local failure modes of a process run.
///
/// Kept private and translated to a typed [`RunError`] at the port boundary via
/// [`From<ProcessError>`](RunError). It exists so the `From` impl lands on a *local* type (the orphan
/// rule forbids `impl From<std::io::Error> for RunError` here) and so every host failure — spawn,
/// I/O, non-zero exit, timeout, over-cap — funnels through one typed translation, never a panic.
#[derive(Debug)]
enum ProcessError {
    /// The child could not be spawned (e.g. the interpreter binary was not found).
    Spawn(std::io::Error),
    /// An I/O error reading a pipe or writing stdin, or a missing piped stream.
    Io(std::io::Error),
    /// The child ran to completion with a non-zero exit code, or was terminated by a signal
    /// (`code == None`).
    NonZeroExit {
        /// The exit code, or `None` when the child was killed by a signal.
        code: Option<i32>,
    },
    /// The child exceeded its per-task `timeout` and was cancelled.
    Timeout {
        /// The elapsed budget, in milliseconds.
        ms: u64,
    },
    /// Captured stdout or stderr exceeded the runner's cap.
    OutputTooLarge {
        /// The cap that was exceeded, in bytes.
        cap_bytes: u64,
    },
}

impl ProcessError {
    /// A missing-pipe I/O error naming the stream — an internal invariant breach surfaced as a typed
    /// error rather than an `unwrap` panic.
    fn pipe(stream: &str) -> Self {
        ProcessError::Io(std::io::Error::other(format!(
            "child {stream} pipe was not captured"
        )))
    }
}

impl From<std::io::Error> for ProcessError {
    fn from(error: std::io::Error) -> Self {
        ProcessError::Io(error)
    }
}

impl From<ProcessError> for RunError {
    fn from(error: ProcessError) -> Self {
        match error {
            ProcessError::Spawn(source) => RunError::run_failure(
                "process_spawn_failed",
                format!("failed to spawn process: {source}"),
            ),
            ProcessError::Io(source) => {
                RunError::run_failure("process_io_failed", format!("process I/O failed: {source}"))
            }
            ProcessError::NonZeroExit { code } => RunError::run_failure(
                "process_exit_nonzero",
                match code {
                    Some(code) => format!("process exited with non-zero status {code}"),
                    None => "process was terminated by a signal".to_string(),
                },
            ),
            ProcessError::Timeout { ms } => RunError::run_failure(
                "task_timeout",
                format!("process exceeded its {ms} ms timeout and was cancelled"),
            ),
            ProcessError::OutputTooLarge { cap_bytes } => RunError::run_failure(
                "output_too_large",
                format!("captured output exceeded the {cap_bytes}-byte cap"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use tmx_core::error::ErrorCategory;

    /// A bare `exec` spec running `command`, no env/args/stdin/timeout.
    fn exec(command: &str) -> ProcessSpec {
        ProcessSpec {
            kind: ProcessKind::Exec,
            command: command.to_string(),
            language: None,
            args: Vec::new(),
            env: IndexMap::new(),
            cwd: None,
            stdin: None,
            timeout: None,
        }
    }

    /// A bare `run` spec running `command` in `language` (None → default bash).
    fn run(command: &str, language: Option<&str>) -> ProcessSpec {
        ProcessSpec {
            kind: ProcessKind::Run,
            command: command.to_string(),
            language: language.map(str::to_string),
            args: Vec::new(),
            env: IndexMap::new(),
            cwd: None,
            stdin: None,
            timeout: None,
        }
    }

    #[test]
    fn invocation_maps_exec_and_run_arms() {
        // exec is always `sh -c <command>`, ignoring language.
        let (program, args) = OsProcessRunner::invocation(&exec("echo hi"));
        assert_eq!(program, "sh", "exec runs through sh");
        assert_eq!(
            args,
            vec!["-c".to_string(), "echo hi".to_string()],
            "exec passes the command line to -c"
        );

        // run with no language defaults to bash and, for a non-file command, takes the inline arm.
        let (program, args) = OsProcessRunner::invocation(&run("echo hi", None));
        assert_eq!(program, DEFAULT_RUN_LANGUAGE, "run defaults to bash");
        assert_eq!(
            args,
            vec!["-c".to_string(), "echo hi".to_string()],
            "an inline (non-file) run command takes the -c arm"
        );

        // an explicit, non-empty language overrides the default.
        let (program, _args) = OsProcessRunner::invocation(&run("echo hi", Some("python3")));
        assert_eq!(
            program, "python3",
            "an explicit language wins over the default"
        );
    }

    #[test]
    fn invocation_file_arm_runs_the_path_as_a_script() {
        // A `run` command that names an existing file is the *file* arm: `<interp> <path>`, no -c.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("tmx-invocation-{}.sh", std::process::id()));
        std::fs::write(&path, b"#!/usr/bin/env bash\necho file\n").expect("write temp script");
        let path_str = path.to_str().expect("temp path is utf-8").to_string();

        let (program, args) = OsProcessRunner::invocation(&run(&path_str, None));
        assert_eq!(
            program, DEFAULT_RUN_LANGUAGE,
            "file arm still defaults to bash"
        );
        assert_eq!(
            args,
            vec![path_str],
            "an existing-file command is passed as a script path, not via -c"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn process_error_maps_to_typed_run_errors() {
        // Every host-failure mode is a RunFailure with its own stable, snake_case code — never a
        // panic, never an untyped string.
        let nonzero: RunError = ProcessError::NonZeroExit { code: Some(3) }.into();
        assert_eq!(
            nonzero.category,
            ErrorCategory::RunFailure,
            "non-zero is a run failure"
        );
        assert_eq!(
            nonzero.code, "process_exit_nonzero",
            "non-zero exit has its own code"
        );

        let over_cap: RunError = ProcessError::OutputTooLarge { cap_bytes: 16 }.into();
        assert_eq!(
            over_cap.category,
            ErrorCategory::RunFailure,
            "over-cap is a run failure"
        );
        assert_eq!(
            over_cap.code, "output_too_large",
            "the over-cap code is the named limit code"
        );

        let timed_out: RunError = ProcessError::Timeout { ms: 50 }.into();
        assert_eq!(timed_out.code, "task_timeout", "a timeout has its own code");

        let spawn: RunError = ProcessError::Spawn(std::io::Error::other("boom")).into();
        assert_eq!(
            spawn.code, "process_spawn_failed",
            "a spawn failure has its own code"
        );
    }

    #[tokio::test]
    async fn exec_zero_exit_captures_stdout() {
        let runner = OsProcessRunner::new();
        let output = runner
            .run(exec("printf 'hello'"))
            .await
            .expect("a zero-exit command succeeds");
        assert_eq!(output.exit_code, Some(0), "a clean command exits zero");
        assert_eq!(output.stdout, b"hello", "stdout is captured verbatim");
        assert!(output.stderr.is_empty(), "nothing was written to stderr");
    }

    #[tokio::test]
    async fn exec_non_zero_exit_is_a_typed_error() {
        let runner = OsProcessRunner::new();
        let error = runner
            .run(exec("exit 3"))
            .await
            .expect_err("a non-zero exit is a typed error, not a zero-exit Ok");
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "non-zero exit is a run failure"
        );
        assert_eq!(
            error.code, "process_exit_nonzero",
            "with the non-zero-exit code"
        );
    }

    #[tokio::test]
    async fn exec_forwards_stdin_to_the_child() {
        let runner = OsProcessRunner::new();
        let mut spec = exec("cat");
        spec.stdin = Some("ping".to_string());
        let output = runner.run(spec).await.expect("cat echoes its stdin");
        assert_eq!(output.exit_code, Some(0), "cat exits zero");
        assert_eq!(
            output.stdout, b"ping",
            "stdin is forwarded and echoed to stdout"
        );
    }

    #[tokio::test]
    async fn run_defaults_to_bash() {
        // `[[ ... ]]` and $BASH_VERSION are bash-isms: under POSIX sh they would fail or be empty.
        // Seeing "bash" proves the default interpreter resolved to bash, not sh.
        let runner = OsProcessRunner::new();
        let output = runner
            .run(run(
                r#"if [[ -n "$BASH_VERSION" ]]; then echo bash; else echo other; fi"#,
                None,
            ))
            .await
            .expect("the default bash interpreter runs the inline script");
        assert_eq!(
            output.exit_code,
            Some(0),
            "the script exits zero under bash"
        );
        assert_eq!(output.stdout, b"bash\n", "the run default resolved to bash");
    }

    #[tokio::test]
    async fn run_inline_script_in_explicit_language() {
        // The inline arm with an explicit interpreter (sh here) runs `<interp> -c <command>`.
        let runner = OsProcessRunner::new();
        let output = runner
            .run(run("echo hi", Some("sh")))
            .await
            .expect("an explicit-language inline script runs");
        assert_eq!(output.exit_code, Some(0), "the inline script exits zero");
        assert_eq!(output.stdout, b"hi\n", "the inline arm captured stdout");
    }

    #[tokio::test]
    async fn run_executes_a_script_file() {
        // The file arm: a `command` naming a real file is run as `bash <path>`, not via -c.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("tmx-run-file-{}.sh", std::process::id()));
        std::fs::write(&path, b"echo from-file\n").expect("write temp script");
        let path_str = path.to_str().expect("temp path is utf-8").to_string();

        let runner = OsProcessRunner::new();
        let output = runner
            .run(run(&path_str, None))
            .await
            .expect("a script file runs to completion");
        assert_eq!(output.exit_code, Some(0), "the script file exits zero");
        assert_eq!(
            output.stdout, b"from-file\n",
            "the file arm captured the script's stdout"
        );

        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn timeout_cancels_and_reports() {
        // A command that would run far longer than its budget is cancelled and reported as a typed
        // timeout — and it returns promptly, not after the full sleep.
        let runner = OsProcessRunner::new();
        let mut spec = exec("sleep 30");
        spec.timeout = Some(Milliseconds(50));
        let started = Instant::now();
        let error = runner
            .run(spec)
            .await
            .expect_err("an over-budget command is cancelled, not awaited");
        assert_eq!(error.code, "task_timeout", "the failure names the timeout");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the timeout fired promptly rather than waiting out the full sleep"
        );
    }

    #[tokio::test]
    async fn over_cap_output_is_bounded() {
        // With a tiny cap, a command emitting more than the cap fails with output_too_large rather
        // than buffering unboundedly.
        let runner = OsProcessRunner::with_output_cap_bytes(8);
        let error = runner
            .run(exec("printf '0123456789'"))
            .await
            .expect_err("output beyond the cap is rejected");
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "over-cap is a run failure"
        );
        assert_eq!(
            error.code, "output_too_large",
            "and reports the output_too_large code"
        );
    }

    #[tokio::test]
    async fn under_cap_output_is_allowed() {
        // Negative-space companion: output at or below the cap is *not* rejected, so the bound is a
        // ceiling, not an off-by-one that trips on legitimate output.
        let runner = OsProcessRunner::with_output_cap_bytes(8);
        let output = runner
            .run(exec("printf '01234567'"))
            .await
            .expect("exactly-cap output is allowed");
        assert_eq!(
            output.stdout, b"01234567",
            "cap-sized output is captured, not rejected"
        );
        assert_eq!(output.exit_code, Some(0), "the command still exits zero");
    }
}
