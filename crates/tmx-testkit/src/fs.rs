//! [`MemFileSystem`] — the in-memory [`FileSystem`] fake.
//!
//! Stands in for `file` with no disk: a `path -> bytes` map behind a [`Mutex`], serving every
//! [`FileOp`] (read/write/append/delete/copy/move/exists) against memory. A missing path on a read,
//! delete, copy, or move is a typed [`RunError`] — the negative space a real filesystem would raise
//! as "not found" — never a panic.

use std::collections::BTreeMap;
use std::sync::Mutex;

use tmx_core::RunError;
use tmx_core::ports::driven::{FileOp, FileResult, FileSystem};

/// An in-memory [`FileSystem`]: paths map to byte contents, ordered for deterministic iteration.
#[derive(Debug, Default)]
pub struct MemFileSystem {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemFileSystem {
    /// An empty filesystem.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a file at `path` with `contents` before the run (builder form).
    #[must_use]
    pub fn with_file(self, path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        if let Ok(mut files) = self.files.lock() {
            files.insert(path.into(), contents.into());
        }
        self
    }

    /// Whether a file exists at `path`, for post-run assertions.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.files
            .lock()
            .map(|f| f.contains_key(path))
            .unwrap_or(false)
    }
}

/// The typed error a memory op raises when `path` is absent — mirrors a real "not found".
fn not_found(path: &str) -> RunError {
    RunError::run_failure("file_not_found", format!("no such file in memory: {path}"))
}

/// A poisoned in-memory lock is an internal fault, surfaced as a typed error rather than a panic.
fn lock_poisoned() -> RunError {
    RunError::run_failure(
        "fs_lock_poisoned",
        "the in-memory filesystem lock was poisoned",
    )
}

#[async_trait::async_trait]
impl FileSystem for MemFileSystem {
    async fn op(&self, op: FileOp) -> Result<FileResult, RunError> {
        let mut files = self.files.lock().map_err(|_| lock_poisoned())?;
        match op {
            FileOp::Read { path, encoding: _ } => {
                let contents = files.get(&path).ok_or_else(|| not_found(&path))?.clone();
                Ok(FileResult::Read { contents })
            }
            FileOp::Write { path, contents } => {
                files.insert(path, contents);
                Ok(FileResult::Done)
            }
            FileOp::Append { path, contents } => {
                files.entry(path).or_default().extend(contents);
                Ok(FileResult::Done)
            }
            FileOp::Delete { path } => {
                files.remove(&path).ok_or_else(|| not_found(&path))?;
                Ok(FileResult::Done)
            }
            FileOp::Copy { from, to } => {
                let contents = files.get(&from).ok_or_else(|| not_found(&from))?.clone();
                files.insert(to, contents);
                Ok(FileResult::Done)
            }
            FileOp::Move { from, to } => {
                let contents = files.remove(&from).ok_or_else(|| not_found(&from))?;
                files.insert(to, contents);
                Ok(FileResult::Done)
            }
            FileOp::Exists { path } => Ok(FileResult::Exists {
                exists: files.contains_key(&path),
            }),
        }
    }
}
