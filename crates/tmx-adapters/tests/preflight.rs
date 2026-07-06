//! Reviewable evidence for task 15: preflight driven over the **real** [`FileSourceLoader`],
//! [`FileReferenceResolver`], and [`JsonSchemaValidator`] against the example corpus.
//!
//! This is the O1/O4 reviewable pass over actual files on disk: `docs/examples/folder-layout/`
//! assembles into an ordered [`ResolvedFlow`] with its sibling `environment.*`/`context.*` folded
//! into shared context (`task-1` before `task-2` by natural filename order) plus a [`CapabilitySet`];
//! the single-file `docs/examples/single-file-flow.yaml` still preflights after the loader/validator
//! compose behind preflight (the regression check); a directory with one malformed task aborts before
//! anything runs; and a Flow whose `fetch` port is unwired is `missing_capability`.
//!
//! Async ports are driven with a no-op waker (`block_on_ready`): the file-backed loader/resolver read
//! synchronously, so every future is immediately ready and no async runtime is linked — the same
//! purity-preserving pattern the rest of the workspace uses.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::task::{Context, Poll};

use tmx_adapters::loader::FileSourceLoader;
use tmx_adapters::resolve::FileReferenceResolver;
use tmx_adapters::validate::JsonSchemaValidator;
use tmx_core::{
    AvailableCapabilities, Capability, ErrorCategory, PreflightPorts, PreflightTarget, Preflighted,
    RunError, preflight,
};

/// Drive an immediately-ready future to completion with a no-op waker.
fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("a file-backed preflight future must be immediately ready"),
    }
}

/// The `docs/examples/` directory, resolved from this crate's manifest dir (process-cwd independent).
fn examples_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Some(root) = manifest_dir.parent().and_then(Path::parent) else {
        panic!(
            "workspace root is two levels above {}",
            manifest_dir.display()
        );
    };
    root.join("docs/examples")
}

/// Every immediate file entry of `dir`, as absolute path strings.
fn dir_entries(dir: &Path) -> Vec<String> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) => panic!("read directory {}: {e}", dir.display()),
    };
    let mut entries = Vec::new();
    for entry in read {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => panic!("read dir entry under {}: {e}", dir.display()),
        };
        if path.is_file() {
            entries.push(path.to_string_lossy().into_owned());
        }
    }
    entries
}

/// Preflight `target` with references resolved relative to `base_dir`, over the real adapters.
fn preflight_target(
    target: &PreflightTarget,
    base_dir: &Path,
    available: &AvailableCapabilities,
) -> Result<Preflighted, RunError> {
    let resolver = FileReferenceResolver::new(base_dir);
    let loader = FileSourceLoader::new();
    let schema = match JsonSchemaValidator::new() {
        Ok(schema) => schema,
        Err(e) => panic!("embedded schemas compile: {e}"),
    };
    let ports = PreflightPorts {
        reference_resolver: &resolver,
        source_loader: &loader,
        schema: &schema,
    };
    block_on_ready(preflight(target, ports, available))
}

#[test]
fn folder_layout_assembles_in_natural_order_with_shared_env_and_context() {
    let dir = examples_dir().join("folder-layout");
    let target = PreflightTarget::Directory {
        entries: dir_entries(&dir),
    };
    let out = preflight_target(&target, &dir, &AvailableCapabilities::all())
        .expect("the folder-layout directory preflights");

    // The two task files order naturally: task-1 (fetch-config) before task-2 (write-report).
    assert_eq!(
        out.flow
            .tasks
            .iter()
            .filter_map(|t| t.name.as_deref())
            .collect::<Vec<_>>(),
        vec!["fetch-config", "write-report"],
        "natural filename order puts task-1 before task-2"
    );
    // The sibling environment.toml + context.yaml are folded into shared env/context.
    assert!(
        out.flow.environment.is_some(),
        "environment.toml became the shared environment"
    );
    assert!(
        out.flow.context.is_some(),
        "context.yaml became the shared context"
    );
    // fetch → Http, file → File, the API_KEY env-secret → Secret, the create hook's exec → Process.
    for required in [
        Capability::Http,
        Capability::File,
        Capability::Secret,
        Capability::Process,
    ] {
        assert!(
            out.capabilities.contains(required),
            "the folder-layout requires {required:?}: {:?}",
            out.capabilities
        );
    }
    assert!(
        !out.capabilities.contains(Capability::Store),
        "the folder-layout uses no object store"
    );
}

#[test]
fn single_file_flow_still_preflights_after_the_loader_and_validator_compose() {
    // Regression (certificate §Regression check): the flagship single-file Flow still yields a
    // ResolvedFlow once the real loader/validator/resolver sit behind preflight — now including
    // chasing its `error` hook *reference* to a sibling Flow file (03 §Reference resolution). The
    // corpus example points its error hook at `./hooks/on-error.yaml` (an illustrative external
    // Flow); stage the example plus that hook in a temp dir so the reference resolves over the real
    // FileReferenceResolver and the referenced hook body is inlined before the run.
    let temp = std::env::temp_dir().join(format!("tmx-single-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(temp.join("hooks")).expect("create temp hooks dir");
    std::fs::copy(
        examples_dir().join("single-file-flow.yaml"),
        temp.join("single-file-flow.yaml"),
    )
    .expect("copy the flagship example flow");
    std::fs::write(
        temp.join("hooks/on-error.yaml"),
        "tasks:\n  - name: notify-failure\n    type: fetch\n    with:\n      url: https://hooks.example.com/error\n      method: POST\n",
    )
    .expect("write the referenced error-hook flow");

    let target = PreflightTarget::File("single-file-flow.yaml".to_string());
    let out = preflight_target(&target, &temp, &AvailableCapabilities::all())
        .expect("single-file-flow.yaml preflights");

    assert_eq!(
        out.flow.name.as_deref(),
        Some("build-and-publish"),
        "the resolved flow keeps its declared name"
    );
    assert!(
        out.flow.tasks.len() >= 5,
        "the single-file flow resolves its full task list, got {}",
        out.flow.tasks.len()
    );
    // The provider environment (aws-ecs) and the store/chat tasks pull their ports into the set.
    assert!(
        out.capabilities.contains(Capability::Provider)
            && out.capabilities.contains(Capability::Store)
            && out.capabilities.contains(Capability::Chat),
        "the provider environment and store/chat tasks are required: {:?}",
        out.capabilities
    );

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn a_directory_with_one_malformed_task_aborts_before_any_task_runs() {
    // Negative space: a task file with an unknown `type` fails schema validation in preflight, so the
    // directory aborts with a Validation error and nothing is executed.
    let temp = std::env::temp_dir().join(format!("tmx-preflight-malformed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).expect("create temp dir");
    std::fs::write(
        temp.join("task-1.json"),
        r#"{ "kind": "task", "name": "ok", "type": "exec", "with": { "command": "make" } }"#,
    )
    .expect("write good task");
    std::fs::write(
        temp.join("task-2.json"),
        r#"{ "kind": "task", "name": "bad", "type": "not-a-real-task-type", "with": {} }"#,
    )
    .expect("write malformed task");

    let target = PreflightTarget::Directory {
        entries: dir_entries(&temp),
    };
    let err = preflight_target(&target, &temp, &AvailableCapabilities::all())
        .expect_err("a malformed task aborts the directory preflight");
    assert_eq!(
        err.category,
        ErrorCategory::Validation,
        "a schema-invalid task is a Validation error"
    );
    assert_eq!(err.code, "schema_invalid", "the schema-rejection code");

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn an_unwired_fetch_port_is_missing_capability_naming_the_port_and_task_type() {
    // The folder-layout needs a fetch (HttpClient); with that port unwired, preflight fails up front
    // with a missing_capability naming both the port and the task type.
    let dir = examples_dir().join("folder-layout");
    let target = PreflightTarget::Directory {
        entries: dir_entries(&dir),
    };
    let available = AvailableCapabilities::all().without(Capability::Http);
    let err = preflight_target(&target, &dir, &available)
        .expect_err("an unwired fetch capability fails preflight");
    assert_eq!(
        err.category,
        ErrorCategory::Environment,
        "a missing capability is an Environment error"
    );
    assert_eq!(err.code, "missing_capability", "the capability-check code");
    assert!(
        err.message.contains("HttpClient"),
        "the error names the HttpClient port: {}",
        err.message
    );
    assert_eq!(
        err.task.as_deref(),
        Some("fetch"),
        "the error names the fetch task type"
    );
}
