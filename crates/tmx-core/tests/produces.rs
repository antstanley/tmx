// This whole crate is test code: an `expect`/`unwrap` in a free helper here IS the assertion and its
// panic IS the failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items,
// not an integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Integration tests for the runtime `produces` conformance check (Task 28 O2) — the three states of
//! `--check-produces` over a task whose output violates its `produces` schema, driven through the
//! `EngineRunFlow` use case over the in-memory fake port bundle.
//!
//! The seeded [`FakeSchemaValidator`] returns an error diagnostic from every `validate_produces`, so a
//! task carrying a `produces` schema "violates" it. The three states must differ: `strict` fails the
//! task (terminal `Failed`), a bare `--check-produces` / `warn` emits a warning but the run continues
//! (`Ok`), and an absent flag skips the check entirely (the validator is never called).

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::ports::driven::{ProcessOutput, SourceKind};
use tmx_core::ports::driving::{RunFlow, RunOptions};
use tmx_core::{
    Diagnostic, EngineRunFlow, Milliseconds, ProducesCheck, RunConfig, RunRecord, RunStatus,
    Severity,
};
use tmx_testkit::{
    FakeChatModel, FakeHttpClient, FakeReferenceResolver, FakeSchemaValidator, FakeSecretResolver,
    FakeSourceLoader, FixedClock, MemFileSystem, MemObjectStore, RecordingEventSink,
    RecordingProcessRunner, SeededIdGenerator,
};

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

/// A single-`exec`-task Flow whose task declares a `produces` schema; the seeded validator reports it
/// as violated, so the three `--check-produces` states are exercised over the same Flow.
fn produces_flow() -> Value {
    json!({
        "name": "build-flow",
        "tasks": [
            {
                "name": "build",
                "type": "exec",
                "with": { "command": "echo out" },
                "produces": { "type": "object", "properties": { "count": { "type": "number" } } }
            }
        ]
    })
}

/// Run `produces_flow` under `mode`, returning the terminal record and the number of
/// `validate_produces` calls the seeded validator saw.
fn run_under(mode: ProducesCheck) -> (RunRecord, usize) {
    let process = RecordingProcessRunner::new();
    process.push_result(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"out".to_vec(),
        stderr: Vec::new(),
        ms: Milliseconds(0),
    }));
    let http = FakeHttpClient::new();
    let fs = MemFileSystem::new();
    let store = MemObjectStore::new();
    let chat = FakeChatModel::new();
    let clock = FixedClock::new();
    let events = RecordingEventSink::new();
    let secrets = FakeSecretResolver::new();
    // A seeded error diagnostic makes every `validate_produces` report a mismatch.
    let schema = FakeSchemaValidator::new().with_diagnostic(Diagnostic::new(
        Severity::Error,
        "produces_mismatch",
        "seeded produces violation",
    ));
    let refs = FakeReferenceResolver::new().with_reference(
        "build-flow",
        "build-flow.yaml",
        SourceKind::Yaml,
    );
    let loader = FakeSourceLoader::new().with_source("build-flow.yaml", produces_flow());
    let ids = SeededIdGenerator::new();

    let ports = tmx_core::Ports {
        process: &process,
        http: &http,
        file: &fs,
        store: &store,
        chat: &chat,
        clock: &clock,
        events: &events,
        secrets: &secrets,
        schema: &schema,
        reference_resolver: &refs,
        source_loader: &loader,
    };
    let config = RunConfig {
        check_produces: mode,
        ..RunConfig::default()
    };
    let use_case = EngineRunFlow::new(ports, &ids, config);
    let record = block_on_ready(use_case.run("build-flow", json!({}), RunOptions::default()))
        .expect("the run completes to a terminal record");
    (record, schema.produces_call_count())
}

#[test]
fn absent_flag_skips_the_produces_check_entirely() {
    // Off (absent flag): the task runs, the run is Ok, and `validate_produces` is never called —
    // outputs are not checked at run time.
    let (record, calls) = run_under(ProducesCheck::Off);
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "the run succeeds with no check"
    );
    assert_eq!(
        calls, 0,
        "the runtime produces check is never reached when the flag is absent"
    );
}

#[test]
fn bare_check_produces_warns_but_the_run_continues() {
    // Warn (bare `--check-produces`): the check runs (the validator is called), but a mismatch is a
    // non-blocking warning — the task still succeeds and the run reaches Ok.
    let (record, calls) = run_under(ProducesCheck::Warn);
    assert_eq!(
        record.status,
        RunStatus::Ok,
        "a warn-level mismatch does not fail the run"
    );
    assert_eq!(calls, 1, "the produces check ran once under warn");
}

#[test]
fn strict_check_produces_fails_the_violating_task() {
    // Strict (`--check-produces=strict`): a mismatch fails the task, so the run is terminal Failed and
    // the failing task result carries the produces_mismatch error.
    let (record, calls) = run_under(ProducesCheck::Strict);
    assert_eq!(
        record.status,
        RunStatus::Failed,
        "a strict mismatch fails the task and the run"
    );
    assert_eq!(calls, 1, "the produces check ran once under strict");
    let failed = record
        .results
        .iter()
        .find(|r| r.error.is_some())
        .expect("the strict run records a failing task");
    assert_eq!(
        failed.error.as_ref().map(|e| e.code),
        Some("produces_mismatch"),
        "the failure carries the produces_mismatch code"
    );
}

#[test]
fn the_three_states_are_mutually_distinct() {
    // The negative-space cross-check: the three states are not the same outcome — strict alone fails,
    // and off alone skips the check. This pins that no two states collapse into one behaviour.
    let (off, off_calls) = run_under(ProducesCheck::Off);
    let (warn, warn_calls) = run_under(ProducesCheck::Warn);
    let (strict, _strict_calls) = run_under(ProducesCheck::Strict);

    assert_eq!(off.status, RunStatus::Ok);
    assert_eq!(warn.status, RunStatus::Ok);
    assert_eq!(strict.status, RunStatus::Failed);
    // Off vs warn differ precisely in whether the check ran, even though both end Ok.
    assert_eq!(off_calls, 0, "off never checks");
    assert_eq!(warn_calls, 1, "warn checks");
}
