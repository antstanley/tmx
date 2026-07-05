#![forbid(unsafe_code)]
//! `tmx` — the command-line binary and composition root.
//!
//! The single driving adapter for the runtime. Its job (from the CLI tasks, 17 onward): parse
//! arguments (`clap`), compose the concrete `tmx-adapters` implementations into the `tmx-core` use
//! cases, dispatch the requested command, and — as the *only* place this mapping lives — translate
//! an `ErrorCategory` into a process exit code. No business logic lives here; it parses, composes,
//! calls a use case, and serialises.
//!
//! Depends on `tmx-adapters`, `tmx-core`, and `tmx-schema`.

fn main() {
    // Composition root and command dispatch land in later tasks (17 onward). This scaffold only
    // establishes the binary and its place at the top of the dependency graph.
}
