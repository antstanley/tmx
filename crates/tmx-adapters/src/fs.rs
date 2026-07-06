//! The local-filesystem [`FileSystem`] adapter — the `file` executor.
//!
//! [`LocalFileSystem`] performs the seven `file` operations behind the `FileSystem` port
//! ([`.specs/06-ports-and-adapters.md` §Executor ports](../../../../.specs/06-ports-and-adapters.md)):
//! `read` / `write` / `append` / `delete` / `copy` / `move` / `exists`, against the host's local
//! filesystem via [`std::fs`].
//!
//! A `read` decodes its raw bytes per the requested `encoding` (`utf-8` / `binary` pass the bytes
//! through, `base64` re-encodes them) and is bounded by
//! [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES) (overridable per
//! adapter for tests): a file larger than the cap is a typed `output_too_large` error before it is
//! fully buffered. Every host failure — a missing path, a permission denial, any other
//! [`std::io::Error`] — is routed through [`From<FsError>`](RunError) into a typed [`RunError`],
//! **never** a panic (06 §Adapters return typed errors, never panic on host failure).
//!
//! The adapter reaches only for [`std::fs`], so it carries no async-runtime or heavy-I/O edge; it is
//! gated behind the `fs` Cargo feature so a minimal build can drop it.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use async_trait::async_trait;

use tmx_core::error::RunError;
use tmx_core::ports::driven::{FileOp, FileResult, FileSystem};
use tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES;

/// The `utf-8` encoding token — raw bytes are passed through unchanged (the default when `encoding`
/// is unset). An encoding *identifier*, not a numeric bound, so it lives here, not in
/// `tmx-schema::limits`.
const ENCODING_UTF8: &str = "utf-8";
/// The `binary` encoding token — raw bytes are passed through unchanged.
const ENCODING_BINARY: &str = "binary";
/// The `base64` encoding token — raw bytes are re-encoded as standard base64.
const ENCODING_BASE64: &str = "base64";

/// The number of raw bytes packed into one base64 group (three bytes → four ASCII characters).
const BASE64_BYTES_PER_GROUP: usize = 3;
/// The number of ASCII characters one base64 group emits.
const BASE64_CHARS_PER_GROUP: usize = 4;
/// The standard base64 alphabet (RFC 4648, table 1) — index by a 6-bit value.
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// The base64 pad character emitted for the bytes a final short group did not have.
const BASE64_PAD: u8 = b'=';

/// Performs `file` operations against the host's local filesystem — the built-in [`FileSystem`]
/// adapter.
///
/// Holds only the captured-output cap a `read` enforces; the default is
/// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES). Tests construct one
/// with a tiny cap via [`with_output_cap_bytes`](LocalFileSystem::with_output_cap_bytes) to exercise
/// the over-cap path without producing a 64 MiB file.
#[derive(Debug, Clone)]
pub struct LocalFileSystem {
    /// The captured-output ceiling, in bytes, applied to a `read`.
    output_cap_bytes: u64,
}

impl Default for LocalFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalFileSystem {
    /// A filesystem adapter that bounds a `read` by
    /// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
    #[must_use]
    pub fn new() -> Self {
        Self {
            output_cap_bytes: CAPTURED_OUTPUT_MAX_BYTES,
        }
    }

    /// An adapter with an explicit captured-output cap, in bytes — for tests exercising the
    /// `output_too_large` path with a small file.
    #[must_use]
    pub fn with_output_cap_bytes(output_cap_bytes: u64) -> Self {
        Self { output_cap_bytes }
    }

    /// Perform one filesystem operation, returning a [`FileResult`] or a typed [`FsError`].
    fn perform(&self, op: FileOp) -> Result<FileResult, FsError> {
        match op {
            FileOp::Read { path, encoding } => {
                let raw = self.read_capped(&path)?;
                let contents = encode(raw, encoding.as_deref())?;
                Ok(FileResult::Read { contents })
            }
            FileOp::Write { path, contents } => {
                std::fs::write(&path, &contents).map_err(|source| FsError::io(&path, source))?;
                Ok(FileResult::Done)
            }
            FileOp::Append { path, contents } => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|source| FsError::io(&path, source))?;
                file.write_all(&contents)
                    .map_err(|source| FsError::io(&path, source))?;
                Ok(FileResult::Done)
            }
            FileOp::Delete { path } => {
                std::fs::remove_file(&path).map_err(|source| FsError::io(&path, source))?;
                Ok(FileResult::Done)
            }
            FileOp::Copy { from, to } => {
                std::fs::copy(&from, &to).map_err(|source| FsError::io(&from, source))?;
                Ok(FileResult::Done)
            }
            FileOp::Move { from, to } => {
                std::fs::rename(&from, &to).map_err(|source| FsError::io(&from, source))?;
                Ok(FileResult::Done)
            }
            // `exists` answers a boolean and never fails: an unreadable/permission-blocked path is
            // reported as absent (`false`), never an error, so this arm cannot panic.
            FileOp::Exists { path } => Ok(FileResult::Exists {
                exists: Path::new(&path).exists(),
            }),
        }
    }

    /// Read `path`'s bytes, bounded by the adapter's cap. At most `cap + 1` bytes are pulled so an
    /// over-cap file is detected without ever buffering it whole; exceeding the cap is a typed
    /// [`FsError::OutputTooLarge`], not an out-of-memory buffer.
    fn read_capped(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let file = File::open(path).map_err(|source| FsError::io(path, source))?;
        let limit = self.output_cap_bytes.saturating_add(1);
        let mut buf = Vec::new();
        file.take(limit)
            .read_to_end(&mut buf)
            .map_err(|source| FsError::io(path, source))?;
        if buf.len() as u64 > self.output_cap_bytes {
            return Err(FsError::OutputTooLarge {
                cap_bytes: self.output_cap_bytes,
            });
        }
        Ok(buf)
    }
}

#[async_trait]
impl FileSystem for LocalFileSystem {
    async fn op(&self, op: FileOp) -> Result<FileResult, RunError> {
        // The work is synchronous `std::fs`; no runtime is awaited. The `async` boundary is the port
        // contract, so the body completes on the first poll.
        self.perform(op).map_err(RunError::from)
    }
}

/// Re-encode raw file bytes per the requested `encoding`. `utf-8` / `binary` (and an unset encoding)
/// pass the bytes through unchanged; `base64` re-encodes them as standard base64 ASCII. Any other
/// token is a typed [`FsError::UnknownEncoding`].
fn encode(raw: Vec<u8>, encoding: Option<&str>) -> Result<Vec<u8>, FsError> {
    match encoding {
        None | Some(ENCODING_UTF8) | Some(ENCODING_BINARY) => Ok(raw),
        Some(ENCODING_BASE64) => Ok(base64_encode(&raw).into_bytes()),
        Some(other) => Err(FsError::UnknownEncoding {
            encoding: other.to_string(),
        }),
    }
}

/// Encode `bytes` as standard base64 (RFC 4648, `=`-padded). Pure and allocation-bounded: the output
/// is exactly `4 · ceil(len / 3)` ASCII characters.
#[must_use]
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(
        bytes.len().div_ceil(BASE64_BYTES_PER_GROUP) * BASE64_CHARS_PER_GROUP,
    );
    for group in bytes.chunks(BASE64_BYTES_PER_GROUP) {
        // Pack up to three bytes into a 24-bit big-endian buffer; missing bytes read as zero.
        let b0 = u32::from(group[0]);
        let b1 = group.get(1).map_or(0, |&b| u32::from(b));
        let b2 = group.get(2).map_or(0, |&b| u32::from(b));
        let packed = (b0 << 16) | (b1 << 8) | b2;
        let idx = [
            (packed >> 18) & 0x3f,
            (packed >> 12) & 0x3f,
            (packed >> 6) & 0x3f,
            packed & 0x3f,
        ];
        // The final short group emits a pad character for each byte it did not have.
        let emit = group.len() + 1;
        for (i, &six) in idx.iter().enumerate() {
            let ch = if i < emit {
                BASE64_ALPHABET[six as usize]
            } else {
                BASE64_PAD
            };
            out.push(char::from(ch));
        }
    }
    out
}

/// A typed filesystem failure, translated to a [`RunError`] at the port boundary.
#[derive(Debug)]
enum FsError {
    /// An [`std::io::Error`] from a filesystem call, carrying the path it concerned.
    Io {
        /// The path the failing call operated on.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A `read` whose captured bytes exceeded the adapter's cap.
    OutputTooLarge {
        /// The cap, in bytes, that was exceeded.
        cap_bytes: u64,
    },
    /// A `read` whose `encoding` was not one of `utf-8` / `base64` / `binary`.
    UnknownEncoding {
        /// The unrecognised encoding token.
        encoding: String,
    },
}

impl FsError {
    /// Wrap an [`std::io::Error`] with the path it concerned.
    fn io(path: &str, source: std::io::Error) -> Self {
        FsError::Io {
            path: path.to_string(),
            source,
        }
    }
}

impl From<FsError> for RunError {
    fn from(err: FsError) -> Self {
        match err {
            FsError::Io { path, source } => {
                let error = if source.kind() == ErrorKind::NotFound {
                    RunError::run_failure("file_not_found", format!("file not found: {path}"))
                } else {
                    RunError::run_failure(
                        "file_io_failed",
                        format!("filesystem I/O failed for {path}: {source}"),
                    )
                };
                error.with_path(path)
            }
            FsError::OutputTooLarge { cap_bytes } => RunError::run_failure(
                "output_too_large",
                format!("file read exceeds the captured-output cap of {cap_bytes} bytes"),
            ),
            FsError::UnknownEncoding { encoding } => RunError::validation(
                "unknown_file_encoding",
                format!("unknown file encoding {encoding:?}; expected utf-8, base64, or binary"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::task::{Context, Poll};

    use tmx_core::error::ErrorCategory;

    /// Drive a port future to completion. The adapter's body is synchronous, so it is `Ready` on the
    /// first poll — a noop waker is sufficient (no runtime needed in this feature-light crate).
    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a synchronous filesystem op must complete on first poll"),
        }
    }

    /// A unique temp path for `tag`, isolated per test and per process. Removed by the caller.
    fn temp_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("tmx-fs-{tag}-{}-{n}", std::process::id()))
    }

    fn path_str(path: &Path) -> String {
        path.to_str().expect("temp path is utf-8").to_string()
    }

    #[test]
    fn write_then_read_round_trips_as_utf8() {
        let path = temp_path("write-read");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        let done = block_on_ready(fs.op(FileOp::Write {
            path: p.clone(),
            contents: b"hello world".to_vec(),
        }))
        .expect("write succeeds");
        assert_eq!(done, FileResult::Done, "write reports Done");

        let read = block_on_ready(fs.op(FileOp::Read {
            path: p.clone(),
            encoding: Some(ENCODING_UTF8.to_string()),
        }))
        .expect("read succeeds");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"hello world".to_vec()
            },
            "read returns exactly what was written, utf-8 bytes unchanged"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn append_extends_and_creates_when_absent() {
        let path = temp_path("append");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        // Append to an absent path creates it.
        block_on_ready(fs.op(FileOp::Append {
            path: p.clone(),
            contents: b"one".to_vec(),
        }))
        .expect("append creates the file");
        block_on_ready(fs.op(FileOp::Append {
            path: p.clone(),
            contents: b"-two".to_vec(),
        }))
        .expect("append extends the file");

        let read = block_on_ready(fs.op(FileOp::Read {
            path: p.clone(),
            encoding: None,
        }))
        .expect("read succeeds");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"one-two".to_vec()
            },
            "append concatenates in order, not overwrites"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn delete_removes_the_file() {
        let path = temp_path("delete");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        std::fs::write(&path, b"gone soon").expect("seed the file");
        assert!(path.exists(), "precondition: the file is present");

        let done =
            block_on_ready(fs.op(FileOp::Delete { path: p.clone() })).expect("delete succeeds");
        assert_eq!(done, FileResult::Done, "delete reports Done");
        assert!(!path.exists(), "the file is gone after delete");

        let exists =
            block_on_ready(fs.op(FileOp::Exists { path: p })).expect("exists never errors");
        assert_eq!(
            exists,
            FileResult::Exists { exists: false },
            "exists reports the deleted path as absent"
        );
    }

    #[test]
    fn copy_duplicates_leaving_the_source() {
        let from = temp_path("copy-src");
        let to = temp_path("copy-dst");
        let fs = LocalFileSystem::new();

        std::fs::write(&from, b"payload").expect("seed the source");
        block_on_ready(fs.op(FileOp::Copy {
            from: path_str(&from),
            to: path_str(&to),
        }))
        .expect("copy succeeds");

        assert!(from.exists(), "copy leaves the source in place");
        assert_eq!(
            std::fs::read(&to).expect("read the copy"),
            b"payload",
            "the destination holds the copied bytes"
        );

        std::fs::remove_file(&from).ok();
        std::fs::remove_file(&to).ok();
    }

    #[test]
    fn move_renames_removing_the_source() {
        let from = temp_path("move-src");
        let to = temp_path("move-dst");
        let fs = LocalFileSystem::new();

        std::fs::write(&from, b"relocate").expect("seed the source");
        block_on_ready(fs.op(FileOp::Move {
            from: path_str(&from),
            to: path_str(&to),
        }))
        .expect("move succeeds");

        assert!(!from.exists(), "move removes the source");
        assert_eq!(
            std::fs::read(&to).expect("read the moved file"),
            b"relocate",
            "the destination holds the moved bytes"
        );

        std::fs::remove_file(&to).ok();
    }

    #[test]
    fn exists_reports_present_and_absent() {
        let path = temp_path("exists");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        let absent =
            block_on_ready(fs.op(FileOp::Exists { path: p.clone() })).expect("exists never errors");
        assert_eq!(
            absent,
            FileResult::Exists { exists: false },
            "an absent path reports false"
        );

        std::fs::write(&path, b"here").expect("create the file");
        let present =
            block_on_ready(fs.op(FileOp::Exists { path: p })).expect("exists never errors");
        assert_eq!(
            present,
            FileResult::Exists { exists: true },
            "a present path reports true"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_base64_encodes_the_raw_bytes() {
        let path = temp_path("base64");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        // Non-UTF-8 raw bytes: base64 is the encoding that carries them safely.
        std::fs::write(&path, [0xff_u8, 0x00, 0x10]).expect("seed raw bytes");
        let read = block_on_ready(fs.op(FileOp::Read {
            path: p,
            encoding: Some(ENCODING_BASE64.to_string()),
        }))
        .expect("read succeeds");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"/wAQ".to_vec()
            },
            "base64 re-encodes the raw bytes per RFC 4648"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_binary_passes_raw_bytes_through() {
        let path = temp_path("binary");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        std::fs::write(&path, [0x00_u8, 0x01, 0xfe]).expect("seed raw bytes");
        let read = block_on_ready(fs.op(FileOp::Read {
            path: p,
            encoding: Some(ENCODING_BINARY.to_string()),
        }))
        .expect("read succeeds");
        assert_eq!(
            read,
            FileResult::Read {
                contents: vec![0x00, 0x01, 0xfe]
            },
            "binary passes the raw bytes through unchanged"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_path_read_is_a_typed_not_found_error() {
        let path = temp_path("missing");
        let fs = LocalFileSystem::new();

        let error = block_on_ready(fs.op(FileOp::Read {
            path: path_str(&path),
            encoding: None,
        }))
        .expect_err("a missing path is an error, not a panic");
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "a missing path is a typed run failure, never a panic"
        );
        assert_eq!(
            error.code, "file_not_found",
            "the missing-path code is stable and specific"
        );
    }

    #[test]
    fn over_cap_read_is_output_too_large() {
        let path = temp_path("overcap");
        let p = path_str(&path);
        // A 4-byte cap with a 16-byte file forces the over-cap path without a huge file.
        let fs = LocalFileSystem::with_output_cap_bytes(4);

        std::fs::write(&path, b"0123456789abcdef").expect("seed an over-cap file");
        let error = block_on_ready(fs.op(FileOp::Read {
            path: p,
            encoding: None,
        }))
        .expect_err("an over-cap read is an error, not a panic");
        assert_eq!(
            error.code, "output_too_large",
            "an over-cap read reports the named limit code"
        );
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "over-cap is a run failure"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn at_cap_read_is_allowed() {
        let path = temp_path("atcap");
        let p = path_str(&path);
        // Exactly at the cap is allowed; only strictly-over is rejected.
        let fs = LocalFileSystem::with_output_cap_bytes(4);

        std::fs::write(&path, b"abcd").expect("seed an exactly-at-cap file");
        let read = block_on_ready(fs.op(FileOp::Read {
            path: p,
            encoding: None,
        }))
        .expect("an at-cap read is allowed");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"abcd".to_vec()
            },
            "a file exactly at the cap reads back whole"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unknown_encoding_is_a_validation_error() {
        let path = temp_path("badenc");
        let p = path_str(&path);
        let fs = LocalFileSystem::new();

        std::fs::write(&path, b"data").expect("seed the file");
        let error = block_on_ready(fs.op(FileOp::Read {
            path: p,
            encoding: Some("rot13".to_string()),
        }))
        .expect_err("an unknown encoding is rejected, not silently mis-decoded");
        assert_eq!(
            error.category,
            ErrorCategory::Validation,
            "an unknown encoding is a validation error"
        );
        assert_eq!(
            error.code, "unknown_file_encoding",
            "the unknown-encoding code is stable"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn chained_write_read_move_exists_reflects_each_step() {
        // The reviewable flow: write → read → move → exists, observing state after each step.
        let src = temp_path("chain-src");
        let dst = temp_path("chain-dst");
        let src_p = path_str(&src);
        let dst_p = path_str(&dst);
        let fs = LocalFileSystem::new();

        block_on_ready(fs.op(FileOp::Write {
            path: src_p.clone(),
            contents: b"chained".to_vec(),
        }))
        .expect("write");
        let read = block_on_ready(fs.op(FileOp::Read {
            path: src_p.clone(),
            encoding: None,
        }))
        .expect("read");
        assert_eq!(
            read,
            FileResult::Read {
                contents: b"chained".to_vec()
            },
            "the written content reads back"
        );

        block_on_ready(fs.op(FileOp::Move {
            from: src_p.clone(),
            to: dst_p.clone(),
        }))
        .expect("move");

        let src_exists =
            block_on_ready(fs.op(FileOp::Exists { path: src_p })).expect("exists never errors");
        let dst_exists =
            block_on_ready(fs.op(FileOp::Exists { path: dst_p })).expect("exists never errors");
        assert_eq!(
            src_exists,
            FileResult::Exists { exists: false },
            "the source is gone after the move"
        );
        assert_eq!(
            dst_exists,
            FileResult::Exists { exists: true },
            "the destination exists after the move"
        );

        std::fs::remove_file(&dst).ok();
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // The classic RFC 4648 §10 test vectors pin the encoder, including its padding.
        assert_eq!(base64_encode(b""), "", "empty encodes to empty");
        assert_eq!(base64_encode(b"f"), "Zg==", "one byte → two pad chars");
        assert_eq!(base64_encode(b"fo"), "Zm8=", "two bytes → one pad char");
        assert_eq!(base64_encode(b"foo"), "Zm9v", "three bytes → no padding");
        assert_eq!(
            base64_encode(b"foobar"),
            "Zm9vYmFy",
            "six bytes, two full groups"
        );
    }
}
