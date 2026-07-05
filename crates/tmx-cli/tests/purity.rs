//! Workspace purity guard, manifest level (development-guidelines.md §Definition of done).
//!
//! The pure crates — `tmx-schema`, `tmx-core`, `tmx-testkit` — must declare no async-runtime or I/O
//! dependency: that is the hexagon's core-purity boundary (02-crate-architecture.md §Dependency
//! graph). This is the fast, offline, in-`nextest` complement to `scripts/purity.sh`, which checks
//! the full transitive `cargo tree`. It lives in `tmx-cli` (the top of the graph) and reads the
//! sibling manifests directly. Its negative-space companion proves the scanner actually detects a
//! forbidden edge — so the positive test can genuinely fail — and that a comment naming a forbidden
//! crate (several of these manifests carry one) is never mistaken for a dependency.

use std::path::{Path, PathBuf};

/// Crates that MUST stay pure — mirrors `PURE_CRATES` in `scripts/purity.sh`.
const PURE_CRATES: [&str; 3] = ["tmx-schema", "tmx-core", "tmx-testkit"];

/// Dependency names a pure crate must never declare — mirrors `FORBIDDEN_CRATES` in
/// `scripts/purity.sh`. `tokio` is the async runtime, `reqwest` the HTTP client, and the
/// `aws-sdk-s3` / `rust-s3` / `rusoto_s3` / `object_store` family the object store.
const FORBIDDEN_DEPS: [&str; 6] = [
    "tokio",
    "reqwest",
    "aws-sdk-s3",
    "rust-s3",
    "rusoto_s3",
    "object_store",
];

/// The workspace root, derived from this test crate's compile-time manifest dir
/// (`<root>/crates/tmx-cli`), so the test does not depend on the process working directory.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match manifest_dir.parent().and_then(Path::parent) {
        Some(root) => root.to_path_buf(),
        None => panic!(
            "workspace root is two levels above {}",
            manifest_dir.display()
        ),
    }
}

/// The lowercased crate names declared under any `dependencies`-family table of a manifest.
/// Comments are stripped and table context is tracked, so a comment mentioning a forbidden crate is
/// never counted as a dependency edge. Both the `[dependencies]` + `name = …` form and the
/// `[dependencies.name]` table-header form are recognised.
fn declared_dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for raw in manifest.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let segments: Vec<&str> = header.split('.').map(str::trim).collect();
            in_deps = segments.last().is_some_and(|s| s.ends_with("dependencies"));
            let named = segments
                .iter()
                .position(|s| s.ends_with("dependencies"))
                .and_then(|pos| segments.get(pos + 1))
                .map(|s| s.trim_matches(['"', '\'']))
                .filter(|s| !s.is_empty());
            if let Some(name) = named {
                names.push(name.to_ascii_lowercase());
            }
            continue;
        }
        if in_deps {
            let name = line.split(['=', '.', ' ']).next().unwrap_or("").trim();
            if !name.is_empty() {
                names.push(name.to_ascii_lowercase());
            }
        }
    }
    names
}

#[test]
fn pure_crate_manifests_declare_no_io_dependency() {
    let root = workspace_root();
    let mut inspected = 0_usize;
    for crate_name in PURE_CRATES {
        let path = root.join("crates").join(crate_name).join("Cargo.toml");
        let manifest = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => panic!("cannot read {}: {err}", path.display()),
        };
        let deps = declared_dependency_names(&manifest);
        for forbidden in FORBIDDEN_DEPS {
            assert!(
                !deps.iter().any(|dep| dep.as_str() == forbidden),
                "purity: `{crate_name}` declares forbidden I/O/async dependency `{forbidden}`",
            );
        }
        inspected += 1;
    }
    assert_eq!(
        inspected,
        PURE_CRATES.len(),
        "every pure crate manifest must be inspected"
    );
    assert!(
        inspected >= 1,
        "at least one pure crate manifest must exist to inspect"
    );
}

#[test]
fn scanner_detects_a_forbidden_edge_and_ignores_comments() {
    // Negative space: a forbidden dep IS detected, so the positive test above can genuinely fail.
    let dirty =
        "[dependencies]\ntmx-core = { path = \"../tmx-core\" }\ntokio = { version = \"1\" }\n";
    let dirty_deps = declared_dependency_names(dirty);
    assert!(
        dirty_deps.iter().any(|d| d == "tokio"),
        "scanner must detect an injected tokio dep"
    );
    assert!(
        dirty_deps.iter().any(|d| d == "tmx-core"),
        "scanner must detect ordinary path deps"
    );

    // The `[dependencies.name]` table-header form is detected too.
    let table_form = "[dependencies.reqwest]\nversion = \"0.12\"\n";
    let table_deps = declared_dependency_names(table_form);
    assert!(
        table_deps.iter().any(|d| d == "reqwest"),
        "scanner must detect the [dependencies.name] form",
    );

    // A comment naming a forbidden crate is NOT a dependency edge, and a clean table trips nothing.
    let commented =
        "# no tokio, no reqwest here\n[dependencies]\ntmx-schema = { path = \"../tmx-schema\" }\n";
    let clean_deps = declared_dependency_names(commented);
    assert!(
        !clean_deps
            .iter()
            .any(|d| FORBIDDEN_DEPS.contains(&d.as_str())),
        "a comment mentioning a forbidden crate must not count as a dependency",
    );
    assert!(
        clean_deps.iter().any(|d| d == "tmx-schema"),
        "the real path dep is still detected"
    );
}
