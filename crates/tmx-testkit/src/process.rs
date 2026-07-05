//! [`RecordingProcessRunner`] — the scripted, recording [`ProcessRunner`] fake.
//!
//! Stands in for `exec`/`run` with no real child process: it replays a queue of scripted
//! [`ProcessOutput`]/[`RunError`] results in order and records every [`ProcessSpec`] it received, so
//! a test can both drive deterministic outcomes and assert on the exact invocations the runner made.

use std::collections::VecDeque;
use std::sync::Mutex;

use tmx_core::Milliseconds;
use tmx_core::RunError;
use tmx_core::ports::driven::{ProcessOutput, ProcessRunner, ProcessSpec};

/// A [`ProcessRunner`] that replays scripted results and records the specs it was asked to run.
///
/// Scripted results are consumed FIFO. When the script is empty, a default success (exit `0`, empty
/// output, zero duration) is returned, so an unscripted runner still drives a Flow deterministically.
#[derive(Debug, Default)]
pub struct RecordingProcessRunner {
    scripted: Mutex<VecDeque<Result<ProcessOutput, RunError>>>,
    calls: Mutex<Vec<ProcessSpec>>,
}

impl RecordingProcessRunner {
    /// An empty runner: every call returns the default success until results are scripted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next successful invocation to capture `stdout` and exit `0`.
    #[must_use]
    pub fn with_stdout(self, stdout: impl Into<Vec<u8>>) -> Self {
        self.push_result(Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: Vec::new(),
            ms: Milliseconds(0),
        }));
        self
    }

    /// Script the next invocation to fail with `error`.
    #[must_use]
    pub fn with_error(self, error: RunError) -> Self {
        self.push_result(Err(error));
        self
    }

    /// Enqueue one scripted result (shared-reference form, for assembling a runner in place).
    pub fn push_result(&self, result: Result<ProcessOutput, RunError>) {
        if let Ok(mut queue) = self.scripted.lock() {
            queue.push_back(result);
        }
    }

    /// The specs this runner was asked to run, in call order.
    #[must_use]
    pub fn calls(&self) -> Vec<ProcessSpec> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl ProcessRunner for RecordingProcessRunner {
    async fn run(&self, spec: ProcessSpec) -> Result<ProcessOutput, RunError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(spec);
        }
        let scripted = self.scripted.lock().ok().and_then(|mut q| q.pop_front());
        scripted.unwrap_or_else(|| {
            Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
                ms: Milliseconds(0),
            })
        })
    }
}
