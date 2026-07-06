// This whole crate is test code: an `expect`/`unwrap` in a free helper here IS the assertion and its
// panic IS the failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items,
// not an integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Integration tests for the `EngineLintFlow` use case (Task 28 O1) — the deeper static pass
//! (resolution + dataflow) behind `tmx lint`, driven over the in-memory fake resolve/load/validate
//! ports.
//!
//! These exercise lint at its two depths: the pure dataflow pass (a typo'd `produces` read, an
//! undeclared input, an unlisted secret) surfaced through the port path, and the resolution pass (a
//! cyclic `flow` import detected by walking the import graph). Each is a warning [`Diagnostic`]; the
//! `--strict` promotion to an exit-3 error is the CLI's concern, tested there.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::ports::driven::SourceKind;
use tmx_core::ports::driving::LintFlow;
use tmx_core::{Diagnostic, EngineLintFlow, PreflightPorts};
use tmx_testkit::{FakeReferenceResolver, FakeSchemaValidator, FakeSourceLoader};

/// Drive an immediately-ready future to completion with a no-op waker (the purity-preserving pattern).
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake future must be immediately ready"),
    }
}

/// Lint `reference` over a resolver/loader seeded from `sources` (`reference -> (path, value)`), with
/// an always-valid schema validator. Returns the findings.
fn lint(reference: &str, sources: &[(&str, &str, Value)]) -> Vec<Diagnostic> {
    let mut refs = FakeReferenceResolver::new();
    let mut loader = FakeSourceLoader::new();
    for (name, path, value) in sources {
        refs = refs.with_reference(*name, *path, SourceKind::Yaml);
        loader = loader.with_source(*path, value.clone());
    }
    let schema = FakeSchemaValidator::new();
    let ports = PreflightPorts {
        reference_resolver: &refs,
        source_loader: &loader,
        schema: &schema,
    };
    let use_case = EngineLintFlow::new(ports);
    block_on_ready(use_case.lint(reference)).expect("lint runs over a parseable flow")
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn lint_catches_a_typo_undeclared_input_and_unlisted_secret_through_the_ports() {
    // O1: a Flow with a typo'd produces read, an undeclared input, and an unlisted secret — each a
    // distinct warning surfaced by the use case (load → analyse), a depth pure schema validation
    // (which would pass this structurally-valid Flow) never reaches.
    let flow = json!({
        "name": "deploy",
        "inputs": { "name": { "type": "string" } },
        "tasks": [
            {
                "name": "build",
                "type": "exec",
                "with": { "command": "echo ${{ inputs.name }}" },
                "produces": { "type": "object", "properties": { "artifact": { "type": "string" } } }
            },
            {
                "name": "ship",
                "type": "exec",
                "secrets": ["TOKEN"],
                "with": {
                    "command": "deploy ${{ tasks.build.artifcat }} ${{ inputs.missing }} ${{ secrets.OTHER }}"
                }
            }
        ]
    });
    let found = lint("deploy", &[("deploy", "deploy.yaml", flow)]);
    let found = codes(&found);
    assert!(
        found.contains(&"produces_field_unknown"),
        "the typo'd produces read is caught: {found:?}"
    );
    assert!(
        found.contains(&"undeclared_input"),
        "the undeclared input is caught: {found:?}"
    );
    assert!(
        found.contains(&"undeclared_secret"),
        "the unlisted secret is caught: {found:?}"
    );
}

#[test]
fn lint_is_clean_on_a_fully_declared_flow() {
    // Negative space: a Flow whose reads are all declared draws no finding — lint does not cry wolf.
    let flow = json!({
        "name": "deploy",
        "inputs": { "name": { "type": "string" } },
        "tasks": [
            {
                "name": "build",
                "type": "exec",
                "with": { "command": "echo ${{ inputs.name }}" },
                "produces": { "type": "object", "properties": { "artifact": {} } }
            },
            {
                "name": "ship",
                "type": "exec",
                "secrets": ["TOKEN"],
                "with": { "command": "deploy ${{ tasks.build.artifact }} ${{ secrets.TOKEN }}" }
            }
        ]
    });
    let found = lint("deploy", &[("deploy", "deploy.yaml", flow)]);
    assert!(
        found.is_empty(),
        "a fully-declared flow lints clean, got {:?}",
        codes(&found)
    );
}

#[test]
fn lint_detects_a_cyclic_flow_import() {
    // O1: flow A imports B and B imports A — walking the import graph must detect the cycle rather than
    // recurse forever, and report it as a `cyclic_flow_import` finding.
    let flow_a = json!({
        "name": "a",
        "tasks": [ { "name": "call-b", "type": "flow", "with": { "use": "b" } } ]
    });
    let flow_b = json!({
        "name": "b",
        "tasks": [ { "name": "call-a", "type": "flow", "with": { "use": "a" } } ]
    });
    let found = lint("a", &[("a", "a.yaml", flow_a), ("b", "b.yaml", flow_b)]);
    assert!(
        codes(&found).contains(&"cyclic_flow_import"),
        "the A→B→A cycle is detected: {:?}",
        codes(&found)
    );
}

#[test]
fn lint_of_an_acyclic_import_graph_is_clean() {
    // Negative-space companion: A imports B, B imports nothing — no cycle, no finding, and the walk
    // terminates.
    let flow_a = json!({
        "name": "a",
        "tasks": [ { "name": "call-b", "type": "flow", "with": { "use": "b" } } ]
    });
    let flow_b = json!({
        "name": "b",
        "tasks": [ { "name": "noop", "type": "exec", "with": { "command": "true" } } ]
    });
    let found = lint("a", &[("a", "a.yaml", flow_a), ("b", "b.yaml", flow_b)]);
    assert!(
        !codes(&found).contains(&"cyclic_flow_import"),
        "an acyclic import graph reports no cycle: {:?}",
        codes(&found)
    );
}
