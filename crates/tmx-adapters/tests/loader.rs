//! Reviewable evidence for task 13: cross-format parity, `kind` dispatch, referrer-relative
//! resolution, and cyclic-`flow`-import detection — driven through the real [`FileSourceLoader`] and
//! [`FileReferenceResolver`] adapters.
//!
//! Async ports are driven with a no-op waker (`block_on_ready`): the loader/resolver read files
//! synchronously, so every future is immediately ready and no async runtime is pulled in — the same
//! purity-preserving pattern `tmx-core` and `tmx-testkit` use.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::Value;

use tmx_adapters::loader::{
    ArtifactClass, FileSourceLoader, classify_artifact, detect_source_kind,
};
use tmx_adapters::resolve::{FileReferenceResolver, assert_acyclic_flow_imports};
use tmx_core::ports::driven::{ReferenceResolver, SourceLoader};

/// Drive an immediately-ready future to completion with a no-op waker.
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a file-backed loader/resolver future must be immediately ready"),
    }
}

/// The `docs/examples/` directory, resolved from this crate's manifest dir so the test is independent
/// of the process working directory.
fn examples_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(root) = manifest_dir.parent().and_then(Path::parent) else {
        panic!(
            "workspace root is two levels above {}",
            manifest_dir.display()
        );
    };
    root.join("docs").join("examples")
}

/// Load `path` through the real loader, detecting its format from the extension.
fn load(path: &Path) -> Value {
    let Some(path_str) = path.to_str() else {
        panic!("path is not valid UTF-8: {}", path.display());
    };
    let kind = match detect_source_kind(path_str) {
        Ok(kind) => kind,
        Err(e) => panic!("a known extension for {path_str}: {e}"),
    };
    match block_on_ready(FileSourceLoader::new().load(path_str, kind)) {
        Ok(value) => value,
        Err(e) => panic!("the corpus file {path_str} loads: {e}"),
    }
}

#[test]
fn all_four_formats_parse_to_one_identical_model() {
    // The same logical Flow in YAML/JSON/JSONC/TOML must land in one identical model — TMX's defining
    // "four formats, one model" trait. `serde_json::Value` equality compares key/value pairs
    // recursively and order-independently, so it is exactly "same model".
    let dir = examples_dir();
    let yaml = load(&dir.join("single-file-flow.yaml"));
    let json = load(&dir.join("single-file-flow.json"));
    let jsonc = load(&dir.join("single-file-flow.jsonc"));
    let toml = load(&dir.join("single-file-flow.toml"));

    assert_eq!(yaml, json, "YAML and JSON must yield the identical model");
    assert_eq!(json, jsonc, "JSON and JSONC must yield the identical model");
    assert_eq!(
        jsonc, toml,
        "JSONC and TOML must yield the identical model — TOML integer/table typing included"
    );
    // Spot-check the residue's named divergence points: TOML integer vs JSON integer, and the
    // string-typed `timeout` that must not become a number.
    assert_eq!(
        toml["tasks"][4]["with"]["assertions"][0]["expected"], 200,
        "the TOML integer 200 equals the JSON integer 200"
    );
    assert_eq!(
        toml["tasks"][0]["with"]["timeout"], "5m",
        "a quoted duration stays a string in every format"
    );
}

#[test]
fn kind_dispatch_selects_the_right_target_across_the_corpus() {
    let dir = examples_dir();
    let cases = [
        ("single-file-flow.yaml", ArtifactClass::Flow),
        ("provider-manifest.yaml", ArtifactClass::Provider),
        ("folder-layout/environment.toml", ArtifactClass::Environment),
        ("folder-layout/context.yaml", ArtifactClass::Context),
        ("folder-layout/task-1.jsonc", ArtifactClass::Task),
        ("folder-layout/task-2.yaml", ArtifactClass::Task),
    ];
    for (rel, want) in cases {
        let path = dir.join(rel);
        let value = load(&path);
        let got = classify_artifact(path.to_str().expect("utf8"), &value)
            .unwrap_or_else(|e| panic!("{rel} classifies: {e}"));
        assert_eq!(got, want, "{rel} dispatches to {want:?}");
    }
}

/// A unique scratch directory under the cargo-provided per-test temp root.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("task13-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        panic!("create scratch dir {}: {e}", dir.display());
    }
    dir
}

fn write(path: &Path, contents: &str) {
    if let Err(e) = std::fs::write(path, contents) {
        panic!("write fixture file {}: {e}", path.display());
    }
}

#[test]
fn a_reference_resolves_relative_to_its_referrer_not_the_cwd() {
    // The sibling lives ONLY in the referrer's directory; the process cwd (the crate dir) has no
    // `sibling.yaml`. If resolution used the cwd it would fail — that it succeeds proves resolution
    // is anchored to the referring document's directory.
    let dir = scratch("relative");
    let referrer = dir.join("flow.yaml");
    let sibling = dir.join("sibling.yaml");
    write(&referrer, "kind: flow\ntasks: []\n");
    write(&sibling, "kind: flow\ntasks: []\n");

    let resolver = FileReferenceResolver::for_referrer(referrer.to_str().expect("utf8"));
    let resolved =
        block_on_ready(resolver.resolve("./sibling.yaml")).expect("sibling resolves via referrer");

    let want = std::fs::canonicalize(&sibling).expect("canonicalise sibling");
    assert_eq!(
        PathBuf::from(&resolved.path),
        want,
        "resolution lands on the referrer's sibling, not a cwd-relative path"
    );
    assert_eq!(
        resolved.kind,
        tmx_core::ports::driven::SourceKind::Yaml,
        "the resolved source carries its detected format"
    );
}

#[test]
fn an_unreadable_path_and_an_unknown_extension_are_distinct_typed_errors() {
    let dir = scratch("errors");
    let referrer = dir.join("flow.yaml");
    write(&referrer, "kind: flow\ntasks: []\n");
    write(&dir.join("notes.txt"), "not a source\n");
    let resolver = FileReferenceResolver::for_referrer(referrer.to_str().expect("utf8"));

    // A valid extension that points at nothing is `reference_not_found`…
    let missing =
        block_on_ready(resolver.resolve("./missing.yaml")).expect_err("missing is an err");
    assert_eq!(
        missing.code, "reference_not_found",
        "unreadable path is typed"
    );
    assert_eq!(
        missing.category,
        tmx_core::error::ErrorCategory::Resolution,
        "a bad reference is a resolution-category error"
    );

    // …while a file with an unknown extension is `unknown_source_format`, a distinct error, even
    // though the file exists.
    let unknown =
        block_on_ready(resolver.resolve("./notes.txt")).expect_err("unknown extension is an err");
    assert_eq!(
        unknown.code, "unknown_source_format",
        "unknown ext is typed"
    );
}

#[test]
fn a_cyclic_flow_import_returns_a_resolution_error_instead_of_recursing() {
    // a → b → a. Without chain tracking this recurses until the stack overflows; the guard must
    // instead terminate with a typed `cyclic_flow_import`.
    let dir = scratch("cycle");
    let a = dir.join("a.yaml");
    let b = dir.join("b.yaml");
    write(
        &a,
        "kind: flow\ntasks:\n  - name: to-b\n    type: flow\n    with:\n      use: ./b.yaml\n",
    );
    write(
        &b,
        "kind: flow\ntasks:\n  - name: to-a\n    type: flow\n    with:\n      use: ./a.yaml\n",
    );

    let loader = FileSourceLoader::new();
    let err = block_on_ready(assert_acyclic_flow_imports(
        &loader,
        a.to_str().expect("utf8"),
    ))
    .expect_err("a cycle must be reported, not recursed");
    assert_eq!(err.code, "cyclic_flow_import", "the cycle is a typed error");
    assert_eq!(
        err.category,
        tmx_core::error::ErrorCategory::Resolution,
        "a cyclic import is a resolution-category error"
    );
}

#[test]
fn an_acyclic_import_chain_resolves_relative_to_each_referrer() {
    // a → b → c (leaf). Each `use` is relative to its own file's directory; the whole chain must
    // pass the guard, confirming the walk both follows referrer-relative edges and does NOT
    // false-positive on a straight chain.
    let dir = scratch("acyclic");
    let a = dir.join("a.yaml");
    let b = dir.join("b.yaml");
    let c = dir.join("c.yaml");
    write(
        &a,
        "kind: flow\ntasks:\n  - name: to-b\n    type: flow\n    with:\n      use: ./b.yaml\n",
    );
    write(
        &b,
        "kind: flow\ntasks:\n  - name: to-c\n    type: flow\n    with:\n      use: ./c.yaml\n",
    );
    write(
        &c,
        "kind: flow\ntasks:\n  - name: leaf\n    type: exec\n    with:\n      command: true\n",
    );

    let loader = FileSourceLoader::new();
    block_on_ready(assert_acyclic_flow_imports(
        &loader,
        a.to_str().expect("utf8"),
    ))
    .expect("a straight a->b->c chain is acyclic");
}

#[test]
fn the_task_map_form_keeps_source_key_order() {
    // The residue's explicit check: a map whose keys are NOT already sorted. Under an order-losing
    // map the loaded keys would come back alphabetically (alpha, mango, zebra) — the source order
    // (zebra, mango, alpha) proves the loader feeds keys to an order-preserving map.
    let dir = scratch("order");
    let flow = dir.join("map-order.yaml");
    write(
        &flow,
        "kind: flow\ntasks:\n  zebra: echo z\n  mango: echo m\n  alpha: echo a\n",
    );
    let value = load(&flow);
    let keys: Vec<&str> = value["tasks"]
        .as_object()
        .expect("tasks is a map")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec!["zebra", "mango", "alpha"],
        "map-form task keys keep source order, not sorted order"
    );
    assert_ne!(
        keys,
        vec!["alpha", "mango", "zebra"],
        "a sorted result would prove the order was lost"
    );
}
