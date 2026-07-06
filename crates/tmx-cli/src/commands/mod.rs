//! Command modules — one thin module per CLI command (07 §Implementation layout).
//!
//! Each module maps its parsed arguments to a driving-port use case, orchestrates preflight, and
//! returns the terminal record (or a typed error) for `main` to render and map to an exit code. Task
//! 17 lands [`run`]; the remaining commands arrive with their tasks.

pub mod context;
pub mod env;
pub mod fmt;
pub mod init;
pub mod inspect;
pub mod lifecycle;
pub mod lint;
pub mod list;
pub mod provider;
pub mod run;
pub mod runs;
pub mod secrets;
pub mod validate;
pub mod version;
