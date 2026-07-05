#![forbid(unsafe_code)]
//! `tmx-testkit` — the in-memory fake adapters.
//!
//! One deterministic fake per driven port, mirroring `tmx-adapters` but with no real I/O: a
//! strictly serial `Scheduler`, a frozen `Clock`, a seeded `IdGenerator`, and recording stand-ins
//! for the process, HTTP, chat, filesystem, object-store, secret, event-sink, and run-store ports.
//! The core's unit tests, the workspace conformance suite, and downstream embedders inject this one
//! shared fake set instead of the built-in adapters — the determinism payoff of the hexagon.
//!
//! Depends on `tmx-core` and `tmx-schema` only — no `tokio`, no `reqwest`, no I/O crate — so it
//! stays inside the same purity boundary as the core it fakes (the `cargo tree` purity check covers
//! it too). The fakes arrive with task 06.
