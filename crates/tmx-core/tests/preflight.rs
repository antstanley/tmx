//! Integration tests for [`tmx_core::preflight`] — the load → resolve → validate → capability pass —
//! driven over the Task-06 in-memory source/reference/schema fakes.
//!
//! These exercise Task-15's definition of done with no real I/O: a single-file and a directory Flow
//! preflight to a [`Preflighted`] carrying an ordered [`ResolvedFlow`] and a [`CapabilitySet`] (O1);
//! natural filename order puts `task-1` before `task-2` with a sibling `environment.*`/`context.*`
//! folded into shared env/context (O1/O4); a malformed artifact and every breached preflight limit
//! fail fast with the right typed error (O2); and a Flow that needs an unwired port is
//! `missing_capability` naming the port and the task type (O2). The fakes never yield, so a single
//! poll with a no-op waker drives every future — no async runtime is linked, preserving purity.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll};

use serde_json::{Value, json};

use tmx_core::ports::driven::{SourceKind, SourceLoader};
use tmx_core::{
    AvailableCapabilities, Capability, Diagnostic, ErrorCategory, PreflightPorts, PreflightTarget,
    Preflighted, RunError, Severity, preflight,
};
use tmx_testkit::{FakeReferenceResolver, FakeSchemaValidator, FakeSourceLoader};

/// Drive an immediately-ready future to completion with a no-op waker (the workspace purity pattern).
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a fake future must be immediately ready"),
    }
}

/// Seed a reference resolver + source loader so `reference` resolves to `path` (as `kind`) and loads
/// `value`; the basename of `path` is what natural ordering keys on.
fn seed(
    entries: &[(&str, &str, Value)],
    schema: FakeSchemaValidator,
) -> (FakeReferenceResolver, FakeSourceLoader, FakeSchemaValidator) {
    let mut refs = FakeReferenceResolver::new();
    let mut loader = FakeSourceLoader::new();
    for (reference, path, value) in entries {
        refs = refs.with_reference(*reference, *path, SourceKind::Json);
        loader = loader.with_source(*path, value.clone());
    }
    (refs, loader, schema)
}

fn run(
    target: &PreflightTarget,
    refs: &FakeReferenceResolver,
    loader: &FakeSourceLoader,
    schema: &FakeSchemaValidator,
    available: &AvailableCapabilities,
) -> Result<Preflighted, RunError> {
    let ports = PreflightPorts {
        reference_resolver: refs,
        source_loader: loader,
        schema,
    };
    block_on_ready(preflight(target, ports, available))
}

fn exec_task(name: &str, command: &str) -> Value {
    json!({ "name": name, "type": "exec", "with": { "command": command } })
}

#[test]
fn single_flow_file_preflights_to_a_resolved_flow_and_capability_set() {
    // A single Flow file with one exec task passes wholesale: the resolved Flow is ordered and the
    // capability set names exactly the ProcessRunner port.
    let flow = json!({
        "kind": "flow",
        "name": "demo",
        "tasks": [ exec_task("build", "make"), exec_task("test", "make test") ]
    });
    let (refs, loader, schema) = seed(
        &[("flow.json", "/d/flow.json", flow)],
        FakeSchemaValidator::new(),
    );
    let out = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect("a valid single-file flow preflights");

    assert_eq!(out.flow.name.as_deref(), Some("demo"), "flow name resolved");
    assert_eq!(
        out.flow
            .tasks
            .iter()
            .filter_map(|t| t.name.as_deref())
            .collect::<Vec<_>>(),
        vec!["build", "test"],
        "tasks stay in source order"
    );
    assert_eq!(out.capabilities.len(), 1, "one port required");
    assert!(
        out.capabilities.contains(Capability::Process),
        "an exec task requires the ProcessRunner"
    );
    assert!(out.warnings.is_empty(), "a clean flow raises no warnings");
}

#[test]
fn directory_assembles_shared_env_context_and_orders_tasks_naturally() {
    // A directory folds environment.*/context.* into shared env/context and orders the two task
    // files by natural filename order — entries are supplied reversed to prove the sort runs.
    let environment = json!({ "kind": "environment", "name": "dev", "platform": "local" });
    let context = json!({
        "kind": "context",
        "env": { "LOG": "info" },
        "secrets": { "API_KEY": { "env": "API_KEY" } }
    });
    let task_1 = json!({
        "kind": "task", "name": "fetch-config", "type": "fetch",
        "secrets": ["API_KEY"], "with": { "url": "https://x" }
    });
    let task_2 = json!({
        "kind": "task", "name": "write-report", "type": "file",
        "with": { "operation": "write", "path": "out", "content": "hi" }
    });
    let (refs, loader, schema) = seed(
        &[
            ("task-2.yaml", "/d/task-2.yaml", task_2),
            ("task-1.jsonc", "/d/task-1.jsonc", task_1),
            ("environment.toml", "/d/environment.toml", environment),
            ("context.yaml", "/d/context.yaml", context),
        ],
        FakeSchemaValidator::new(),
    );
    // Entry order is deliberately not natural order; preflight must impose it.
    let target = PreflightTarget::Directory {
        entries: vec![
            "task-2.yaml".to_string(),
            "task-1.jsonc".to_string(),
            "environment.toml".to_string(),
            "context.yaml".to_string(),
        ],
    };
    let out = run(
        &target,
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect("a valid directory preflights");

    assert_eq!(
        out.flow
            .tasks
            .iter()
            .filter_map(|t| t.name.as_deref())
            .collect::<Vec<_>>(),
        vec!["fetch-config", "write-report"],
        "task-1 precedes task-2 by natural filename order regardless of entry order"
    );
    assert!(
        out.flow.environment.is_some() && out.flow.context.is_some(),
        "the sibling environment/context are folded into shared context"
    );
    // fetch → Http, file → File, structured secret → Secret; no provider ⇒ no EnvironmentProvider.
    assert!(
        out.capabilities.contains(Capability::Http)
            && out.capabilities.contains(Capability::File)
            && out.capabilities.contains(Capability::Secret),
        "the required ports are computed across tasks and context: {:?}",
        out.capabilities
    );
    assert!(
        !out.capabilities.contains(Capability::Provider),
        "an environment without a provider needs no EnvironmentProvider"
    );
}

#[test]
fn one_malformed_task_aborts_a_directory_before_anything_runs() {
    // Negative space: the schema fake reports every artifact invalid; preflight fails fast with a
    // Validation error and returns nothing — no assembly, no capability set, nothing executed.
    let bad = json!({ "kind": "task", "name": "x", "type": "exec", "with": { "command": "make" } });
    let schema = FakeSchemaValidator::new().with_diagnostic(Diagnostic::new(
        Severity::Error,
        "schema_violation",
        "bad task",
    ));
    let (refs, loader, schema) = seed(&[("t.jsonc", "/d/t.jsonc", bad)], schema);
    let err = run(
        &PreflightTarget::Directory {
            entries: vec!["t.jsonc".to_string()],
        },
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect_err("a malformed task aborts preflight");
    assert_eq!(
        err.category,
        ErrorCategory::Validation,
        "malformed ⇒ Validation"
    );
    assert_eq!(err.code, "schema_invalid", "the schema-rejection code");
}

#[test]
fn missing_capability_names_the_port_and_task_type() {
    // A Flow needing `store` with no ObjectStore wired is an Environment `missing_capability` naming
    // both the port and the task type — the fail-up-front capability guarantee.
    let flow = json!({
        "tasks": [ {
            "name": "upload", "type": "store",
            "with": { "operation": "put", "bucket": "b", "key": "k", "content": "x" }
        } ]
    });
    let (refs, loader, schema) = seed(
        &[("flow.json", "/d/flow.json", flow)],
        FakeSchemaValidator::new(),
    );
    let available = AvailableCapabilities::all().without(Capability::Store);
    let err = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &available,
    )
    .expect_err("an unwired store capability fails preflight");
    assert_eq!(
        err.category,
        ErrorCategory::Environment,
        "missing cap ⇒ Environment"
    );
    assert_eq!(err.code, "missing_capability", "the capability-check code");
    assert!(
        err.message.contains("ObjectStore"),
        "the error names the port: {}",
        err.message
    );
    assert_eq!(
        err.task.as_deref(),
        Some("store"),
        "the error names the task type"
    );
}

#[test]
fn nameless_task_is_validation_and_duplicate_name_is_resolution() {
    // Structural checks: a nameless array-form task is a `missing_task_name` Validation error; two
    // same-named tasks are a `duplicate_task_name` Resolution error (03 §Validation).
    let nameless = json!({ "tasks": [ { "type": "exec", "with": { "command": "make" } } ] });
    let (r1, l1, s1) = seed(
        &[("f.json", "/d/f.json", nameless)],
        FakeSchemaValidator::new(),
    );
    let e1 = run(
        &PreflightTarget::File("f.json".to_string()),
        &r1,
        &l1,
        &s1,
        &AvailableCapabilities::all(),
    )
    .expect_err("a nameless task is rejected");
    assert_eq!(e1.category, ErrorCategory::Validation);
    assert_eq!(e1.code, "missing_task_name");

    let dup = json!({ "tasks": [ exec_task("a", "one"), exec_task("a", "two") ] });
    let (r2, l2, s2) = seed(&[("f.json", "/d/f.json", dup)], FakeSchemaValidator::new());
    let e2 = run(
        &PreflightTarget::File("f.json".to_string()),
        &r2,
        &l2,
        &s2,
        &AvailableCapabilities::all(),
    )
    .expect_err("a duplicate name is rejected");
    assert_eq!(
        e2.category,
        ErrorCategory::Resolution,
        "duplicate ⇒ Resolution"
    );
    assert_eq!(e2.code, "duplicate_task_name");
}

#[test]
fn over_limit_counts_widths_depths_and_concurrency_are_rejected() {
    use tmx_schema::limits::{CONCURRENCY_MAX, FANOUT_WIDTH_MAX, TASKS_PER_FLOW_MAX};

    // too_many_tasks: one past the ceiling.
    let many: Vec<Value> = (0..=TASKS_PER_FLOW_MAX)
        .map(|i| exec_task(&format!("t{i}"), "make"))
        .collect();
    let flow = json!({ "tasks": many });
    let (r, l, s) = seed(&[("f.json", "/d/f.json", flow)], FakeSchemaValidator::new());
    let e = run(
        &PreflightTarget::File("f.json".to_string()),
        &r,
        &l,
        &s,
        &AvailableCapabilities::all(),
    )
    .expect_err("too many tasks");
    assert_eq!(e.code, "too_many_tasks");
    assert_eq!(e.category, ErrorCategory::Validation);

    // concurrency_too_high: a map task with an over-ceiling concurrency.
    let concurrency = json!({ "tasks": [ {
        "name": "fan", "type": "map",
        "with": { "items": [1, 2], "concurrency": CONCURRENCY_MAX + 1,
                  "task": { "type": "exec", "with": { "command": "make" } } }
    } ] });
    let (r, l, s) = seed(
        &[("f.json", "/d/f.json", concurrency)],
        FakeSchemaValidator::new(),
    );
    let e = run(
        &PreflightTarget::File("f.json".to_string()),
        &r,
        &l,
        &s,
        &AvailableCapabilities::all(),
    )
    .expect_err("concurrency too high");
    assert_eq!(e.code, "concurrency_too_high");

    // fanout_too_wide: a literal items array one past the width ceiling.
    let wide_items = vec![Value::Null; (FANOUT_WIDTH_MAX as usize) + 1];
    let wide = json!({ "tasks": [ {
        "name": "fan", "type": "map",
        "with": { "items": wide_items, "task": { "type": "exec", "with": { "command": "make" } } }
    } ] });
    let (r, l, s) = seed(&[("f.json", "/d/f.json", wide)], FakeSchemaValidator::new());
    let e = run(
        &PreflightTarget::File("f.json".to_string()),
        &r,
        &l,
        &s,
        &AvailableCapabilities::all(),
    )
    .expect_err("fan-out too wide");
    assert_eq!(e.code, "fanout_too_wide");

    // json_too_deep: a document nested past the JSON depth bound (built directly, bypassing any
    // parser recursion limit, via the source fake).
    let mut deep = json!("leaf");
    for _ in 0..200 {
        deep = Value::Array(vec![deep]);
    }
    let (r, l, s) = seed(
        &[("deep.json", "/d/deep.json", deep)],
        FakeSchemaValidator::new(),
    );
    let e = run(
        &PreflightTarget::File("deep.json".to_string()),
        &r,
        &l,
        &s,
        &AvailableCapabilities::all(),
    )
    .expect_err("json too deep");
    assert_eq!(e.code, "json_too_deep");
}

#[test]
fn a_referenced_environment_and_context_are_chased_and_inlined() {
    // A single Flow file whose `environment` and `context` are *string references* preflights: the
    // resolver/loader chase each to its target, kind-dispatch + validate it, and inline it. The
    // structured secret in the referenced context pulls SecretResolver into the capability set, and
    // the referenced provider environment pulls in EnvironmentProvider — proving the inlined targets
    // are walked, not dropped.
    let flow = json!({
        "kind": "flow",
        "name": "with-refs",
        "environment": "./environment.toml",
        "context": "./context.yaml",
        "tasks": [ exec_task("build", "make") ]
    });
    let environment = json!({
        "kind": "environment", "name": "prod", "platform": "aws", "provider": "aws-ecs"
    });
    let context = json!({
        "kind": "context",
        "env": { "LOG": "info" },
        "secrets": { "API_KEY": { "env": "API_KEY" } }
    });
    let (refs, loader, schema) = seed(
        &[
            ("flow.json", "/d/flow.json", flow),
            ("./environment.toml", "/d/environment.toml", environment),
            ("./context.yaml", "/d/context.yaml", context),
        ],
        FakeSchemaValidator::new(),
    );
    let out = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect("a flow with referenced env/context preflights");

    assert!(
        out.flow.environment.is_some(),
        "the referenced environment was chased and inlined"
    );
    assert!(
        out.flow.context.is_some(),
        "the referenced context was chased and inlined"
    );
    // exec ⇒ Process, structured secret ⇒ Secret, provider environment ⇒ Provider: all three come
    // from targets reachable only through the references, so this fails if inlining were skipped.
    for required in [
        Capability::Process,
        Capability::Secret,
        Capability::Provider,
    ] {
        assert!(
            out.capabilities.contains(required),
            "the inlined targets require {required:?}: {:?}",
            out.capabilities
        );
    }
}

#[test]
fn a_dangling_environment_reference_is_a_typed_resolution_error() {
    // Negative space: the `environment` reference resolves to nothing (unseeded), so preflight fails
    // with the resolver's typed Resolution error rather than silently proceeding with no environment.
    let flow = json!({
        "kind": "flow",
        "environment": "./missing-environment.toml",
        "tasks": [ exec_task("build", "make") ]
    });
    let (refs, loader, schema) = seed(
        &[("flow.json", "/d/flow.json", flow)],
        FakeSchemaValidator::new(),
    );
    let err = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect_err("a dangling environment reference fails preflight");
    assert_eq!(
        err.category,
        ErrorCategory::Resolution,
        "a dangling reference ⇒ Resolution"
    );
    assert_eq!(
        err.code, "reference_not_found",
        "the resolver's typed not-found code surfaces unchanged"
    );
}

#[test]
fn a_referenced_hook_body_pulls_its_ports_into_the_capability_set() {
    // A context whose `create` hook is a *reference* to a Flow with an exec task must pull the
    // ProcessRunner into the capability set — otherwise a referenced hook needing a port would escape
    // the missing_capability guarantee. Proven by making it fail when ProcessRunner is unwired.
    let flow = json!({
        "kind": "flow",
        "context": { "hooks": { "create": "./hook.yaml" } },
        "tasks": [ { "name": "check", "type": "assert",
                     "with": { "assertions": [
                         { "actual": 1, "matcher": "toBe", "expected": 1 } ] } } ]
    });
    let hook = json!({ "kind": "flow", "tasks": [ exec_task("notify", "echo hi") ] });
    let entries = &[
        ("flow.json", "/d/flow.json", flow),
        ("./hook.yaml", "/d/hook.yaml", hook),
    ];

    // Wired: the referenced hook's exec is required and satisfied.
    let (refs, loader, schema) = seed(entries, FakeSchemaValidator::new());
    let out = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect("a flow whose hook references an exec flow preflights when ProcessRunner is wired");
    assert!(
        out.capabilities.contains(Capability::Process),
        "the referenced hook's exec pulls ProcessRunner into the set: {:?}",
        out.capabilities
    );

    // Unwired: the same referenced hook now trips missing_capability up front.
    let (refs, loader, schema) = seed(entries, FakeSchemaValidator::new());
    let err = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::none(),
    )
    .expect_err("an unwired ProcessRunner fails a hook that references an exec flow");
    assert_eq!(err.category, ErrorCategory::Environment);
    assert_eq!(err.code, "missing_capability");
}

#[test]
fn too_many_hook_tasks_across_a_context_is_rejected() {
    use tmx_schema::limits::HOOK_TASKS_MAX;

    // A context whose hooks together hold one more than HOOK_TASKS_MAX inline tasks is a Validation
    // `too_many_hook_tasks` at preflight — the hook-task budget guard (04 §Limits).
    let over: Vec<Value> = (0..=HOOK_TASKS_MAX)
        .map(|i| exec_task(&format!("h{i}"), "make"))
        .collect();
    let flow = json!({
        "kind": "flow",
        "context": { "hooks": { "create": over } },
        "tasks": [ exec_task("build", "make") ]
    });
    let (refs, loader, schema) = seed(
        &[("flow.json", "/d/flow.json", flow)],
        FakeSchemaValidator::new(),
    );
    let err = run(
        &PreflightTarget::File("flow.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect_err("a hook budget overflow fails preflight");
    assert_eq!(err.category, ErrorCategory::Validation);
    assert_eq!(err.code, "too_many_hook_tasks");
}

#[test]
fn a_bad_reference_propagates_as_a_typed_resolution_error_before_validation() {
    // Negative space: an unseeded reference never loads, so preflight surfaces the resolver's typed
    // error rather than proceeding — nothing is assembled or validated.
    let refs = FakeReferenceResolver::new();
    let loader = FakeSourceLoader::new();
    let schema = FakeSchemaValidator::new();
    let err = run(
        &PreflightTarget::File("missing.json".to_string()),
        &refs,
        &loader,
        &schema,
        &AvailableCapabilities::all(),
    )
    .expect_err("an unresolved reference fails preflight");
    assert_eq!(
        err.category,
        ErrorCategory::Resolution,
        "a bad reference ⇒ Resolution"
    );
    // The loader fake is never reached because resolution fails first.
    let _ = &loader as &dyn SourceLoader;
}
