#![forbid(unsafe_code)]
//! `tmx-adapters` — the built-in driven-port adapters.
//!
//! One concrete implementation per driven port declared by `tmx-core`: process execution, HTTP,
//! filesystem, object store, chat model, secret resolution, source loading, schema validation, the
//! system clock, UUIDv7 generation, and the bounded scheduler. This is the crate that lives
//! *outside* the core's purity boundary — where the async runtime and the heavy dependencies
//! (`tokio`, `reqwest`, the S3 SDK) belong. Each adapter is gated behind a Cargo feature so a
//! minimal or sandboxed build can drop the ones it does not need.
//!
//! Depends on `tmx-core` (the port traits it implements) and `tmx-schema`. The adapters themselves
//! arrive in tasks 13–24.

pub mod clock;
pub mod deny;
pub mod idgen;
pub mod loader;
pub mod report;
pub mod resolve;
pub mod scheduler;
pub mod secret;
pub mod validate;

// The tokio-runtime seam: the OS-process adapter and its `tokio` dependency are confined behind the
// `process` Cargo feature (on by default), so a minimal or sandboxed build can drop the async
// runtime with `--no-default-features`. This is the only module that reaches for `tokio`.
#[cfg(feature = "process")]
pub mod process;

// The HTTP `fetch` adapter and its `reqwest` dependency are confined behind the `http` Cargo feature
// (on by default), so a minimal or sandboxed build can drop the HTTP client with
// `--no-default-features`. This is the only module that reaches for `reqwest`.
#[cfg(feature = "http")]
pub mod http;
