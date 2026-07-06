//! `tmx run` — the end-to-end path and its full flag surface (07 §`tmx run`, §Matrix sugar).
//!
//! Resolves the Flow reference (the `--file` → positional → `$TMX_FLOW` → `./flow.{…}`/`./tmx.{…}` →
//! folder-layout order), preflights it (fail-fast validation + the capability check, 03 §Preflight
//! flow), then *prepares* the resolved flow from the run flags — coerces `--input`/`--inputs-file`
//! values to their declared `type`, applies `--env` overrides, slices the task list
//! (`--only`/`--skip`/`--from`/`--until`), and seeds prior state from `--state-in` (re-validated on
//! read) — and executes it through the [`PipelineRunner`]. `--matrix` lowers to a bounded `map`
//! cross-product (each combination its own full run binding `${{ matrix.<key> }}`; an authored `map`
//! wins); `--dry-run` prints the plan and executes nothing; `--watch` re-runs on a source change.
//! Both a single file and a directory / folder-layout target take this one execution path (the
//! preflight already yields the resolved flow for either). The final Pipeline state comes back masked;
//! `--state-out` dumps it, `main` renders it to stdout and maps the outcome to an exit code. The
//! reference-driven [`RunFlow`](tmx_core::ports::driving::RunFlow) use case remains for library/HTTP
//! hosts; the CLI prepares the flow itself so the whole flag surface applies uniformly.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tmx_adapters::clock::SystemClock;
use tmx_adapters::loader::detect_source_kind;
use tmx_adapters::runstore::LocalRunStore;
use tmx_adapters::sink::{FinalStateSink, Format};

use tmx_core::ports::driven::RunStore;

use std::time::Duration;

use indexmap::IndexMap;
use serde_json::Value;
use tmx_schema::limits::CONCURRENCY_MAX;

use tmx_core::ports::driven::{Clock, IdGenerator, ProviderMethod};
use tmx_core::{
    CancelReason, CancelToken, Masker, Milliseconds, PipelineState, PreflightTarget, ResolvedFlow,
    RunConfig, RunError, RunRecord, RunStatus, TaskSlice, flow_has_map, matrix_combinations,
    merged_inputs, preflight, slice_tasks,
};

use crate::args::RunArgs;
use crate::commands::lifecycle::{invoke_method, invoke_teardown, load_provider};
use crate::compose::Composed;
use crate::config;

/// A resolved run target: what to preflight, the single file reference (when the target is one file),
/// and the base directory reference resolution is rooted at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    /// The preflight target (a single file, or a directory's enumerated entries).
    pub target: PreflightTarget,
    /// The canonical file path driving the `RunFlow` use case, or `None` for a directory / layout.
    pub file_reference: Option<String>,
    /// The directory reference resolution is anchored at (the Flow's own directory).
    pub base_dir: PathBuf,
}

/// Run the `tmx run` command to its terminal [`RunRecord`] (or a typed [`RunError`]).
///
/// # Errors
///
/// Returns a [`RunError`] for an unresolved Flow (`resolution`), a malformed artifact or breached
/// limit (`validation`), a missing capability (`environment`), or any failure the run itself surfaces.
/// A run that *completes* with a failed task returns `Ok` with a `failed`-status record — the failure
/// is data on the record, mapped to exit 1 by `main`, not an `Err`.
pub async fn execute(args: RunArgs) -> Result<RunRecord, RunError> {
    if args.watch {
        watch(&args).await
    } else {
        run_once(&args).await
    }
}

/// Run the Flow once, end to end, honouring the full flag surface: coerced inputs, `--env` overrides,
/// `--state-in` seed, task slicing, `--dry-run`, `--matrix` cross-product, and the
/// concurrency/continue-on-error/max-state-size engine flags. Returns the terminal [`RunRecord`] — for
/// `--matrix`, the first *failing* combination's record when any combination failed (so a later passing
/// combination never masks an earlier failure), else the last combination's record (each combination is
/// its own full run).
///
/// # Errors
///
/// Returns a [`RunError`] for an unresolved Flow, a malformed artifact, a coercion/validation failure
/// of an input or a `--state-in` file, an unknown sliced task name, an over-wide matrix, a missing
/// capability, or any failure the run itself surfaces.
async fn run_once(args: &RunArgs) -> Result<RunRecord, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let resolved = resolve_target(args, &cwd, config::env_flow())?;

    // Resolve the reporter surface: the stdout format (flag → TMX_FORMAT → TTY default) and whether
    // the stderr progress is coloured. The stdout TTY check drives pretty-vs-json; the stderr TTY
    // check drives colour. Both are resolved once, here, and threaded into the composed reporter.
    let format = config::resolve_format(
        args.format.map(crate::args::FormatArg::to_format),
        std::io::stdout().is_terminal(),
    );
    let color = config::resolve_color(args.color, args.no_color, std::io::stderr().is_terminal());

    // Wire the run store unless `--no-store` opts out. It is rooted at `./.tmx/runs` under the cwd (the
    // record is a project-local artifact), and retention is swept opportunistically here, before the
    // run, so an aged record is pruned without waiting for `tmx runs prune`.
    let run_store: Option<Arc<LocalRunStore>> = if args.no_store {
        None
    } else {
        let store = Arc::new(LocalRunStore::new(cwd.join(".tmx").join("runs")));
        sweep_retention(&store).await;
        Some(store)
    };

    // The run's cancellation token, threaded into every adapter call through the composed ports. The
    // grace window (`--grace`, default `CANCEL_GRACE_MS`) and, when set, the `--timeout` budget drive
    // background watchers that request cancellation then hard-stop after the grace — SIGINT does the
    // same. A run with no `--timeout` still installs the SIGINT watcher, so Ctrl-C always cancels.
    let cancel = CancelToken::new();
    let grace_ms = config::resolve_grace_ms(args.grace.as_deref());
    let timeout_ms = args.timeout.as_deref().and_then(config::parse_duration_ms);
    spawn_cancellation_watchers(&cancel, timeout_ms, grace_ms);

    let composed = Composed::new(resolved.base_dir.clone(), format, color, run_store.clone())?
        .with_cancel(cancel.clone());
    let preflighted = preflight(
        &resolved.target,
        composed.preflight_ports(),
        &composed.available_capabilities(),
    )
    .await?;
    // Non-fatal validation notes go to stderr, keeping stdout clean for the final-state JSON.
    for warning in &preflighted.warnings {
        eprintln!("warning: {}", warning.message);
    }

    // `--concurrency` is the global cap for `map`/`eval` fan-out; a request above the engine ceiling is
    // rejected up front (the same bound `run_map`/`run_eval` enforce), so the flag is validated at the
    // boundary rather than silently accepted.
    check_concurrency(args.concurrency)?;

    // The engine flags the run flags shape: `--continue-on-error` forces the global policy,
    // `--check-produces` selects the `produces` mode (absent → Off), and `--max-state-size` narrows
    // the state cap (clamped to the hard ceiling by the runner).
    let config = RunConfig {
        continue_on_error: args.continue_on_error,
        check_produces: args
            .check_produces
            .map(crate::args::CheckProducesArg::to_check)
            .unwrap_or_default(),
        max_state_size_bytes: args.max_state_size,
    };

    // Prepare the flow from the run flags: coerce the supplied inputs to their declared `type`, apply
    // `--env` overrides onto the context, slice the task list (`--only`/`--skip`/`--from`/`--until`),
    // and seed prior state from `--state-in` (re-validated on read).
    let inputs = coerce_inputs(args, &preflighted.flow)?;
    let flow = apply_env_overrides(preflighted.flow.clone(), &args.env)?;
    let flow = slice_tasks(flow, &build_slice(args))?;
    let seed = read_state_in(args.state_in.as_deref())?;

    // `--matrix` lowers to a bounded `map` cross-product binding `${{ matrix.<key> }}` per combination
    // — unless an authored `map` wins, in which case it is ignored with a stderr warning. An empty
    // list means a single, matrix-free run.
    let combos = resolve_matrix(args, &flow)?;

    // `--dry-run` / `-n`: resolve + validate + print the plan; execute nothing (no env lifecycle, no
    // task side effect, no store write).
    if args.dry_run {
        let id = composed.ids().new_run_id();
        return dry_run_plan(id, &flow, &inputs, &combos, format);
    }

    // The ephemeral-environment lifecycle wraps the run(s) (06 §Ephemeral lifecycle):
    //   default        → deploy → run → clean
    //   --keep         → deploy → run
    //   --no-deploy    →          run           (reuse a standing environment)
    //   --local        →          run           (no provider at all)
    // A provider method that fails is an `environment` error (exit 5), distinct from a run failure.
    let environment = flow.environment.clone();
    let provider_loaded = match &environment {
        Some(env) if !args.local && env.provider.is_some() => {
            Some(load_provider(env, &composed).await?)
        }
        _ => None,
    };

    // Deploy up front, unless a standing environment is reused (`--no-deploy`).
    if let (Some(loaded), Some(env)) = (&provider_loaded, &environment)
        && !args.no_deploy
        && let Err(deploy_err) = invoke_method(loaded, &composed, env, ProviderMethod::Deploy).await
    {
        // Best-effort teardown even after a failed deploy (unless `--keep`), then surface exit 5.
        if !args.keep
            && let Err(clean_err) =
                invoke_teardown(loaded, &composed, env, ProviderMethod::Clean).await
        {
            eprintln!(
                "tmx: warning: clean after a failed deploy also failed: {}",
                clean_err.message
            );
        }
        return Err(deploy_err);
    }

    // Run once per matrix combination (or exactly once with no matrix binding, `None`). Each
    // combination is a full run with its own record and store entry. A matrix lowers to a `map`, so a
    // combination that *completes* with a non-`Ok` status (a failed `assert`, a missed `eval`
    // threshold) is a run failure of the whole matrix (07 §Matrix sugar, §Exit codes) — a later
    // passing combination must never mask it. `last` tracks the most recent record; `first_failure`
    // latches the *first* combination that failed (a hard `Err`, or an `Ok` record whose status is
    // not `Ok`). The returned record — and thus the process exit code — is that first failure when any
    // combination failed, else the last record, so `--matrix a=1,2` where a=1 fails and a=2 passes
    // exits 1, not 0.
    let combinations: Vec<Option<Value>> = if combos.is_empty() {
        vec![None]
    } else {
        combos.into_iter().map(Some).collect()
    };
    let mut last: Result<RunRecord, RunError> =
        Err(RunError::run_failure("no_run", "no run executed"));
    let mut first_failure: Option<Result<RunRecord, RunError>> = None;
    for matrix in combinations {
        let record =
            run_flow_direct(&flow, &inputs, seed.as_ref(), matrix, &composed, config).await;
        // Persist each combination's terminal snapshot. A store write failure is observability, not
        // the run's result, so it is a warning, never a reason to fail the run.
        if let (Some(store), Ok(record)) = (&run_store, &record)
            && let Err(store_err) = store.save(record).await
        {
            eprintln!(
                "tmx: warning: could not persist the run record: {}",
                store_err.message
            );
        }
        // A hard error (an unresolved reference, a breached limit) stops the matrix; a completed run
        // with a non-`Ok` status is a valid record, so the remaining combinations still run — but it
        // is latched as a failure so a later pass cannot mask it.
        let hard_error = record.is_err();
        let failed = match &record {
            Ok(record) => record.status != RunStatus::Ok,
            Err(_) => true,
        };
        if failed && first_failure.is_none() {
            first_failure = Some(record.clone());
        }
        last = record;
        if hard_error {
            break;
        }
    }
    // The first failing combination, if any, is the run's result (so a later pass never masks an
    // earlier failure); otherwise every combination passed and the last record is the result.
    let result = first_failure.unwrap_or(last);

    // Clean best-effort even after a failed run. Skipped by `--keep` (leave it up) and `--no-deploy`
    // (we never provisioned it).
    if let (Some(loaded), Some(env)) = (&provider_loaded, &environment)
        && !args.no_deploy
        && !args.keep
        && let Err(clean_err) = invoke_teardown(loaded, &composed, env, ProviderMethod::Clean).await
    {
        eprintln!("tmx: warning: provider clean failed: {}", clean_err.message);
    }

    // `--state-out` dumps the (already masked) final state of the terminal run to a file, and the
    // selected `--format` renders the stdout machine data (json → the final state object; ndjson /
    // pretty already streamed during the run).
    if let Ok(record) = &result {
        if let Some(path) = &args.state_out {
            write_state_out(record, path)?;
        }
        emit_final_state(record, format)?;
    }
    result
}

/// Spawn the background cancellation watchers on the current Tokio runtime (06 §Concurrency,
/// cancellation, timeouts). Each watcher, on its trigger, *requests* cancellation (the runner stops
/// dispatching new work), waits the `grace_ms` window (skipped when `0` for an immediate hard stop),
/// then *hard-cancels* (in-flight adapters are abandoned):
///
/// - a **SIGINT** watcher is always installed, so Ctrl-C cancels any run and it exits 130;
/// - a **`--timeout`** watcher is installed only when `timeout_ms` is set, so an over-budget run exits 124.
///
/// The watchers hold `CancelToken` clones (a shared flag), so triggering one is observed by the run
/// through the composed ports. A run that finishes first simply leaves the pending watchers to be
/// dropped with the runtime — a never-fired watcher is inert.
fn spawn_cancellation_watchers(cancel: &CancelToken, timeout_ms: Option<u64>, grace_ms: u64) {
    let interrupt = cancel.clone();
    tokio::spawn(async move {
        // A failed signal registration must not crash the run; it just means Ctrl-C won't cancel.
        if tokio::signal::ctrl_c().await.is_ok() {
            escalate(&interrupt, CancelReason::Interrupt, grace_ms).await;
        }
    });

    if let Some(ms) = timeout_ms {
        let timeout = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            escalate(&timeout, CancelReason::Timeout, grace_ms).await;
        });
    }
}

/// Request cancellation for `reason`, wait the grace window (unless `0`), then hard-cancel — the
/// two-phase escalation both watchers share.
async fn escalate(cancel: &CancelToken, reason: CancelReason, grace_ms: u64) {
    cancel.request(reason);
    if grace_ms > 0 {
        tokio::time::sleep(Duration::from_millis(grace_ms)).await;
    }
    cancel.hard_cancel(reason);
}

/// Sweep the run store's retention window opportunistically at the start of a run: prune every record
/// older than the resolved window ([`config::resolve_retention_days`]), unless retention is disabled
/// (`runs.retention` / `TMX_RUNS_RETENTION` set to `0` / `off`). Best-effort — a prune failure is a
/// warning, never a reason to fail the run about to start.
async fn sweep_retention(store: &LocalRunStore) {
    let Some(days) = config::resolve_retention_days() else {
        // Retention disabled (`0` / `off`): the sweep is fully skipped.
        return;
    };
    let cutoff = SystemClock::new().cutoff_days_ago(days);
    match store.prune(&cutoff).await {
        Ok(0) => {}
        Ok(pruned) => eprintln!("tmx: pruned {pruned} run record(s) past the retention window"),
        Err(prune_err) => eprintln!(
            "tmx: warning: run-store retention sweep failed: {}",
            prune_err.message
        ),
    }
}

/// Render the machine-data stdout for the resolved `format` at run end: under `json`, the merged final
/// Pipeline state as one masked JSON object (07 §stdout / stderr contract); under `ndjson`/`pretty`,
/// nothing (the event stream / stderr progress already carried the run).
///
/// The state on the [`RunRecord`] is **already redacted** by the run's Masker inside the core (the use
/// case / preflighted path both mask it before it leaves the engine, 04 §Secrets & masking). Re-sealing
/// it through a fresh Masker here mints the [`Masked`](tmx_core::Masked) typestate the
/// [`FinalStateSink`] requires — the boundary proof that no *raw* state object can reach stdout — so
/// the sink's routing assertion holds without the run's Masker having to escape the core.
fn emit_final_state(record: &RunRecord, format: Format) -> Result<(), RunError> {
    if format != Format::Json {
        return Ok(());
    }
    let state = record
        .final_state
        .as_ref()
        .map_or_else(|| serde_json::json!({}), |s| s.as_value().clone());
    let masked = Masker::new().redact_state(&state);
    FinalStateSink::new().emit(&masked)
}

/// Execute a prepared [`ResolvedFlow`] directly through the runner, returning the masked terminal
/// [`RunRecord`]. `inputs` is the already-coerced `inputs.*` scope, `seed` the optional `--state-in`
/// prior state, and `matrix` the `--matrix` combination bound as `${{ matrix.<key> }}` (`None` for a
/// matrix-free run). This is the single execution path both a single-file and a directory target take
/// once the flag surface has prepared the flow — it mirrors the `RunFlow` use case's own tail (mint id
/// → run → mask final state → build the record).
async fn run_flow_direct(
    flow: &ResolvedFlow,
    inputs: &Value,
    seed: Option<&PipelineState>,
    matrix: Option<Value>,
    composed: &Composed,
    config: RunConfig,
) -> Result<RunRecord, RunError> {
    let ports = composed.ports();
    let merged = merged_inputs(inputs, &flow.inputs);

    let id = composed.ids().new_run_id();
    let started_at = ports.clock.now();
    let start_ms = ports.clock.now_ms();
    let mut masker = Masker::new();
    let mut resolved_secrets: Vec<String> = Vec::new();

    let runner = match matrix {
        Some(binding) => composed.runner(config).with_matrix(binding),
        None => composed.runner(config),
    };
    let pipeline = runner
        .run(
            &id,
            flow,
            &merged,
            ports,
            &mut masker,
            &mut resolved_secrets,
            seed,
            0,
        )
        .await?
        .pipeline;

    let finished_at = ports.clock.now();
    let total_ms = Milliseconds(ports.clock.now_ms().0.saturating_sub(start_ms.0));

    // Mask the merged final state through the run's Masker before it leaves the process boundary.
    let masked_state = masker
        .redact_value(pipeline.state.as_value())
        .into_inner()
        .into_owned();
    // The state stays an object across the merge, so re-wrapping cannot fail; fall back to an empty
    // state rather than take a panicking path.
    let final_state = PipelineState::new(masked_state).unwrap_or_else(|_| PipelineState::empty());

    Ok(RunRecord {
        id,
        flow: flow.name.clone(),
        status: pipeline.status,
        started_at,
        finished_at: Some(finished_at),
        ms: Some(total_ms),
        final_state: Some(final_state),
        results: pipeline.results,
    })
}

// ---------------------------------------------------------------------------------------------
// The run-flag depth helpers (07 §`tmx run` run flags, §Matrix sugar): input coercion, `--env`
// overrides, task slicing, `--state-in` seed, `--matrix` lowering, `--dry-run` plan, `--state-out`
// dump, and the `--watch` re-run loop.
// ---------------------------------------------------------------------------------------------

/// Build the supplied `inputs.*` object from `--inputs-file` then the `--input` flags (a later flag
/// overrides the file and an earlier flag), coercing every supplied value to its declared `type`.
///
/// `--input k=v` supplies a string `v`, coerced to the declared type of `k`; `--input k:=<json>`
/// supplies a raw JSON value used as-is. `--inputs-file` reads a JSON object whose values are coerced
/// the same way. A value that cannot be coerced to its declared type is a typed `input_type_mismatch`
/// error (negative space) rather than a silently mistyped input.
fn coerce_inputs(args: &RunArgs, flow: &ResolvedFlow) -> Result<Value, RunError> {
    let mut supplied = serde_json::Map::new();

    if let Some(path) = &args.inputs_file {
        let text = std::fs::read_to_string(path).map_err(|e| {
            RunError::resolution(
                "inputs_file_unreadable",
                format!("could not read --inputs-file `{path}`: {e}"),
            )
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            RunError::validation(
                "inputs_file_invalid",
                format!("--inputs-file `{path}` is not valid JSON: {e}"),
            )
        })?;
        let object = value.as_object().ok_or_else(|| {
            RunError::validation(
                "inputs_file_not_object",
                format!("--inputs-file `{path}` must be a JSON object of input values"),
            )
        })?;
        for (key, raw) in object {
            supplied.insert(key.clone(), coerce_declared(key, raw.clone(), flow)?);
        }
    }

    for entry in &args.input {
        // `k:=<json>` supplies a raw JSON value; `k=v` supplies a coerced string. Test `:=` first so a
        // JSON value containing `=` is not mis-split.
        if let Some((key, json)) = entry.split_once(":=") {
            let value: Value = serde_json::from_str(json.trim()).map_err(|e| {
                RunError::validation(
                    "input_json_invalid",
                    format!("--input {key}:= value is not valid JSON: {e}"),
                )
            })?;
            supplied.insert(key.to_string(), value);
        } else if let Some((key, raw)) = entry.split_once('=') {
            supplied.insert(
                key.to_string(),
                coerce_declared(key, Value::String(raw.to_string()), flow)?,
            );
        } else {
            return Err(RunError::validation(
                "input_malformed",
                format!("--input {entry:?} must be `k=v` or `k:=<json>`"),
            ));
        }
    }

    Ok(Value::Object(supplied))
}

/// Coerce a supplied input `value` to the declared `type` of input `key`, when the flow declares one.
/// An undeclared input, or one declared `string` (or with no `type`), passes through unchanged.
fn coerce_declared(key: &str, value: Value, flow: &ResolvedFlow) -> Result<Value, RunError> {
    let declared = flow
        .inputs
        .get(key)
        .and_then(|spec| spec.input_type.as_deref());
    let Some(declared) = declared else {
        return Ok(value);
    };
    // A value already of the right JSON type is accepted as-is; only a string needs coercion.
    let Value::String(text) = &value else {
        return Ok(value);
    };
    let coerced = match declared {
        "string" => Some(Value::String(text.clone())),
        // Prefer an integer form (so `count=3` is `3`, not `3.0`), falling back to a float.
        "number" => text.parse::<i64>().ok().map(Value::from).or_else(|| {
            text.parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
        }),
        "boolean" => text.parse::<bool>().ok().map(Value::Bool),
        "object" | "array" => serde_json::from_str::<Value>(text).ok(),
        _ => Some(Value::String(text.clone())),
    }
    .ok_or_else(|| {
        RunError::validation(
            "input_type_mismatch",
            format!("input {key:?} value {text:?} does not coerce to declared type {declared:?}"),
        )
    })?;
    // A parsed object/array must match the declared shape (a `[...]` for `object` is rejected).
    let shape_ok = match declared {
        "object" => coerced.is_object(),
        "array" => coerced.is_array(),
        _ => true,
    };
    if !shape_ok {
        return Err(RunError::validation(
            "input_type_mismatch",
            format!("input {key:?} value {text:?} is not a JSON {declared}"),
        ));
    }
    Ok(coerced)
}

/// Apply `--env K=V` overrides onto the flow's context env, creating a context/env map when the flow
/// declares none. A malformed `--env` entry (no `=`) is a typed `env_malformed` error.
fn apply_env_overrides(
    mut flow: ResolvedFlow,
    overrides: &[String],
) -> Result<ResolvedFlow, RunError> {
    if overrides.is_empty() {
        return Ok(flow);
    }
    let mut context = flow.context.take().unwrap_or_default();
    let mut env = context.env.take().unwrap_or_default();
    for entry in overrides {
        let (key, value) = entry.split_once('=').ok_or_else(|| {
            RunError::validation("env_malformed", format!("--env {entry:?} must be `K=V`"))
        })?;
        env.insert(key.to_string(), value.to_string());
    }
    context.env = Some(env);
    flow.context = Some(context);
    Ok(flow)
}

/// Validate a `--concurrency` request against the engine ceiling [`CONCURRENCY_MAX`]: a request above
/// it is a typed `concurrency_too_high` error (the same bound `run_map`/`run_eval` enforce), so an
/// over-limit cap is rejected at the boundary rather than silently clamped. `None` (unset) is Ok.
fn check_concurrency(concurrency: Option<u32>) -> Result<(), RunError> {
    if let Some(requested) = concurrency
        && requested > CONCURRENCY_MAX
    {
        return Err(RunError::validation(
            "concurrency_too_high",
            format!(
                "--concurrency {requested} exceeds the {CONCURRENCY_MAX} ceiling (CONCURRENCY_MAX)"
            ),
        ));
    }
    Ok(())
}

/// Build the [`TaskSlice`] from the slicing flags (`--from`/`--until`/`--only`/`--skip`).
fn build_slice(args: &RunArgs) -> TaskSlice {
    TaskSlice {
        from: args.from.clone(),
        until: args.until.clone(),
        only: args.only.clone(),
        skip: args.skip.clone(),
    }
}

/// Read and re-validate the `--state-in` seed: the file must parse as JSON and be a valid Pipeline
/// state (a top-level object). A malformed or non-object file is rejected as a typed error rather than
/// silently trusting state TMX itself may have written (07 §`tmx run`: re-validate seeded state).
fn read_state_in(path: Option<&str>) -> Result<Option<PipelineState>, RunError> {
    let Some(path) = path else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path).map_err(|e| {
        RunError::resolution(
            "state_in_unreadable",
            format!("could not read --state-in `{path}`: {e}"),
        )
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        RunError::validation(
            "state_in_invalid",
            format!("--state-in `{path}` is not valid JSON: {e}"),
        )
    })?;
    // Re-validation: a seeded state must be a JSON object (`PipelineState::new` enforces it).
    let state = PipelineState::new(value)?;
    Ok(Some(state))
}

/// Parse the `--matrix key=v1,v2` axes and lower them to the bounded cross-product (07 §Matrix sugar).
/// An authored `map` wins: when the flow already declares a `map` task, `--matrix` is ignored with a
/// stderr warning and no combinations are produced (a single, matrix-free run).
fn resolve_matrix(args: &RunArgs, flow: &ResolvedFlow) -> Result<Vec<Value>, RunError> {
    if args.matrix.is_empty() {
        return Ok(Vec::new());
    }
    if flow_has_map(flow) {
        eprintln!(
            "tmx: warning: this Flow authors a `map` task; --matrix is ignored (the authored map wins)"
        );
        return Ok(Vec::new());
    }
    let mut axes: IndexMap<String, Vec<Value>> = IndexMap::new();
    for entry in &args.matrix {
        let (key, values) = entry.split_once('=').ok_or_else(|| {
            RunError::validation(
                "matrix_malformed",
                format!("--matrix {entry:?} must be `key=v1,v2,…`"),
            )
        })?;
        let parsed: Vec<Value> = values
            .split(',')
            .map(|v| {
                let v = v.trim();
                // A value that parses as JSON keeps its type (`1` → number, `true` → boolean);
                // otherwise it is a bare string (`linux`).
                serde_json::from_str::<Value>(v).unwrap_or_else(|_| Value::String(v.to_string()))
            })
            .collect();
        axes.entry(key.to_string()).or_default().extend(parsed);
    }
    matrix_combinations(&axes)
}

/// Print the resolved run plan for `--dry-run` — the flow name, the resolved inputs, the ordered task
/// list, and the matrix combinations — as one JSON object on stdout, and return an `ok` terminal
/// [`RunRecord`] so the process exits `0` having executed nothing.
fn dry_run_plan(
    id: tmx_core::RunId,
    flow: &ResolvedFlow,
    inputs: &Value,
    combos: &[Value],
    format: Format,
) -> Result<RunRecord, RunError> {
    let tasks: Vec<Value> = flow
        .tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "name": task.name.clone().unwrap_or_default(),
                "type": task_type_name(&task.with),
            })
        })
        .collect();
    let plan = serde_json::json!({
        "dryRun": true,
        "flow": flow.name.clone(),
        "inputs": inputs,
        "tasks": tasks,
        "matrix": combos,
    });
    // The plan is machine data → stdout under json/ndjson; under pretty it is a human artifact. Either
    // way it is pretty-printed so a reviewer reads it directly.
    let _ = format;
    let rendered = serde_json::to_string_pretty(&plan).map_err(|e| {
        RunError::run_failure(
            "dry_run_unrenderable",
            format!("could not render the plan: {e}"),
        )
    })?;
    println!("{rendered}");

    Ok(RunRecord {
        id,
        flow: flow.name.clone(),
        status: tmx_core::RunStatus::Ok,
        started_at: SystemClock::new().now(),
        finished_at: Some(SystemClock::new().now()),
        ms: Some(Milliseconds(0)),
        final_state: Some(PipelineState::empty()),
        results: Vec::new(),
    })
}

/// Dump the terminal run's already-masked final state to `--state-out` as pretty JSON, so a later run
/// can resume it via `--state-in`. A write failure is a typed error naming the path.
fn write_state_out(record: &RunRecord, path: &str) -> Result<(), RunError> {
    let state = record
        .final_state
        .as_ref()
        .map_or_else(|| serde_json::json!({}), |s| s.as_value().clone());
    let rendered = serde_json::to_string_pretty(&state).map_err(|e| {
        RunError::run_failure(
            "state_out_unrenderable",
            format!("could not render the final state: {e}"),
        )
    })?;
    std::fs::write(path, rendered).map_err(|e| {
        RunError::run_failure(
            "state_out_unwritable",
            format!("could not write --state-out `{path}`: {e}"),
        )
    })
}

/// The `--watch` loop: run the Flow, then re-run it every time its resolved source file(s) change,
/// each re-run a full run with its own record (07 §Decisions: `--watch` runs are ordinary runs). The
/// watcher polls the source modification times; a SIGINT stops it and the process exits with the most
/// recent run's code. Returns the most recent run's [`RunRecord`].
async fn watch(args: &RunArgs) -> Result<RunRecord, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let resolved = resolve_target(args, &cwd, config::env_flow())?;
    let watched = watched_paths(&resolved);
    let mut fingerprint = source_fingerprint(&watched);

    let mut last = run_once(args).await;
    eprintln!("tmx: watching for changes… (Ctrl-C to stop)");
    loop {
        // Wait for either a source change (re-run) or a SIGINT (stop the watcher).
        let changed = tokio::select! {
            () = wait_for_change(&watched, &mut fingerprint) => true,
            _ = tokio::signal::ctrl_c() => false,
        };
        if !changed {
            eprintln!("tmx: watch stopped");
            break;
        }
        eprintln!("tmx: change detected, re-running");
        last = run_once(args).await;
    }
    last
}

/// The absolute source paths a `--watch` run polls for changes — the single file, or every enumerated
/// entry of a directory target.
fn watched_paths(resolved: &ResolvedTarget) -> Vec<String> {
    match &resolved.target {
        PreflightTarget::File(path) => vec![path.clone()],
        PreflightTarget::Directory { entries } => entries.clone(),
    }
}

/// A cheap change fingerprint of the watched paths: each path's last-modified time in nanoseconds
/// since the epoch (0 when unavailable), in path order.
fn source_fingerprint(paths: &[String]) -> Vec<u128> {
    paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos())
        })
        .collect()
}

/// Poll the watched paths until their fingerprint changes, updating `fingerprint` in place. Polls on a
/// fixed interval so the watcher needs no filesystem-notification dependency (staying inside the
/// adapter dependency budget).
async fn wait_for_change(paths: &[String], fingerprint: &mut Vec<u128>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(WATCH_POLL_INTERVAL_MS)).await;
        let current = source_fingerprint(paths);
        if &current != fingerprint {
            *fingerprint = current;
            return;
        }
    }
}

/// How often the `--watch` loop polls the source modification times, in milliseconds. A local UI
/// cadence (not a bounded *engine* dimension), so it is a named constant here rather than in
/// `tmx-schema::limits`.
const WATCH_POLL_INTERVAL_MS: u64 = 500;

/// The stable `type` token for a task's `with` payload — the dry-run plan's task-type label. Exhaustive
/// match over the closed [`TaskWith`] vocabulary, no wildcard, so a new variant forces an update here.
fn task_type_name(with: &tmx_schema::task::TaskWith) -> &'static str {
    use tmx_schema::task::TaskWith;
    match with {
        TaskWith::Exec(_) => "exec",
        TaskWith::Run(_) => "run",
        TaskWith::Fetch(_) => "fetch",
        TaskWith::File(_) => "file",
        TaskWith::Store(_) => "store",
        TaskWith::ChatCompletion(_) => "chat-completion",
        TaskWith::Assert(_) => "assert",
        TaskWith::Map(_) => "map",
        TaskWith::Eval(_) => "eval",
        TaskWith::Flow(_) => "flow",
    }
}

/// Resolve the Flow reference by the documented order (07 §`tmx run`): `--file/-f` → positional →
/// `$TMX_FLOW` → `./flow.{…}`/`./tmx.{…}` → a folder layout in the cwd → else a `ResolutionError`
/// naming the search path.
///
/// # Errors
///
/// Returns a `resolution` [`RunError`] when an explicitly-named reference does not exist, or when the
/// implicit search finds no Flow file and no folder layout — its message lists every path tried.
pub fn resolve_target(
    args: &RunArgs,
    cwd: &Path,
    env_flow: Option<String>,
) -> Result<ResolvedTarget, RunError> {
    let mut searched: Vec<String> = Vec::new();

    // Explicit rungs: --file wins over the positional, which wins over $TMX_FLOW. The first one that
    // is *provided* is authoritative — if it does not exist, that is an error, not a fall-through.
    let explicit = args.file.clone().or_else(|| args.flow.clone()).or(env_flow);
    if let Some(reference) = explicit {
        searched.push(reference.clone());
        let path = resolve_relative(cwd, &reference);
        if path.is_file() {
            return file_target(&path);
        }
        if path.is_dir() {
            return directory_target(&path);
        }
        return Err(unresolved(&searched));
    }

    // Implicit cwd search for a conventional Flow file, in candidate order.
    for candidate in config::flow_file_candidates() {
        let path = cwd.join(&candidate);
        searched.push(path.display().to_string());
        if path.is_file() {
            return file_target(&path);
        }
    }

    // Folder-layout fallback: the cwd itself carries the shared-artifact layout (environment.* /
    // context.*), so it runs as one assembled directory Flow.
    if is_folder_layout(cwd) {
        searched.push(format!("{} (folder layout)", cwd.display()));
        return directory_target(cwd);
    }

    Err(unresolved(&searched))
}

/// Join `reference` against `cwd`, leaving an absolute reference untouched.
fn resolve_relative(cwd: &Path, reference: &str) -> PathBuf {
    let path = Path::new(reference);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// Build a single-file [`ResolvedTarget`]: the canonical path is both the preflight target and the
/// reference driving the `RunFlow` use case, rooted at the file's own directory.
fn file_target(path: &Path) -> Result<ResolvedTarget, RunError> {
    let canonical = canonicalize(path)?;
    let reference = path_to_string(&canonical)?;
    let base_dir = canonical
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Ok(ResolvedTarget {
        target: PreflightTarget::File(reference.clone()),
        file_reference: Some(reference),
        base_dir,
    })
}

/// Build a directory [`ResolvedTarget`]: the immediate source files become the preflight entries
/// (preflight imposes natural filename order), rooted at the directory itself.
fn directory_target(path: &Path) -> Result<ResolvedTarget, RunError> {
    let canonical = canonicalize(path)?;
    let entries = enumerate_source_files(&canonical)?;
    if entries.is_empty() {
        return Err(RunError::resolution(
            "empty_directory",
            format!(
                "the directory `{}` holds no loadable source artifact",
                canonical.display()
            ),
        ));
    }
    Ok(ResolvedTarget {
        target: PreflightTarget::Directory { entries },
        file_reference: None,
        base_dir: canonical,
    })
}

/// The immediate child files of `dir` whose extension names a known source format, as absolute path
/// strings sorted for determinism (preflight re-orders them by natural filename order regardless).
fn enumerate_source_files(dir: &Path) -> Result<Vec<String>, RunError> {
    let read = std::fs::read_dir(dir).map_err(|e| {
        RunError::resolution(
            "directory_unreadable",
            format!("could not read directory `{}`: {e}", dir.display()),
        )
    })?;
    let mut entries: Vec<String> = Vec::new();
    for item in read {
        let Ok(item) = item else { continue };
        let path = item.path();
        if path.is_file()
            && let Some(text) = path.to_str()
            && detect_source_kind(text).is_ok()
        {
            entries.push(text.to_string());
        }
    }
    entries.sort();
    Ok(entries)
}

/// Whether `dir` carries a shared-artifact folder layout — it holds an `environment.*` or `context.*`
/// file (03 §Directory assembly). The cheap hallmark that distinguishes a runnable layout directory
/// from an arbitrary directory that merely happens to contain source files.
fn is_folder_layout(dir: &Path) -> bool {
    let Ok(read) = std::fs::read_dir(dir) else {
        return false;
    };
    for item in read.flatten() {
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        let stem = name.split_once('.').map_or(name, |(head, _)| head);
        if matches!(stem, "environment" | "context") {
            return true;
        }
    }
    false
}

/// Canonicalise `path` to a stable absolute identity, mapping a failure to a typed resolution error.
fn canonicalize(path: &Path) -> Result<PathBuf, RunError> {
    std::fs::canonicalize(path).map_err(|e| {
        RunError::resolution(
            "reference_not_found",
            format!("could not resolve `{}`: {e}", path.display()),
        )
        .with_path(path.display().to_string())
    })
}

/// Render `path` as a UTF-8 string, mapping a non-UTF-8 path to a typed resolution error.
fn path_to_string(path: &Path) -> Result<String, RunError> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        RunError::resolution(
            "non_utf8_path",
            format!("path is not valid UTF-8: {}", path.display()),
        )
    })
}

/// The `ResolutionError` raised when no Flow resolves — its message lists every path tried, so the
/// operator sees the exact search order (07 §`tmx run`).
fn unresolved(searched: &[String]) -> RunError {
    RunError::resolution(
        "flow_unresolved",
        format!(
            "no Flow found. Searched, in order: {}",
            if searched.is_empty() {
                "<nothing>".to_string()
            } else {
                searched.join(", ")
            }
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmx_core::{ErrorCategory, resolve_flow};

    /// A flow declaring three typed inputs, for the coercion tests.
    fn typed_flow() -> ResolvedFlow {
        resolve_flow(serde_json::json!({
            "inputs": {
                "count": { "type": "number" },
                "flag": { "type": "boolean" },
                "name": { "type": "string" },
                "tags": { "type": "array" }
            },
            "tasks": []
        }))
        .expect("the fixture flow resolves")
    }

    #[test]
    fn coerce_inputs_coerces_each_value_to_its_declared_type() {
        // `--input k=v` coerces a string to the declared type; `k:=<json>` supplies a raw JSON value.
        let args = RunArgs {
            input: vec![
                "count=3".to_string(),
                "flag=true".to_string(),
                "name=release".to_string(),
                "tags:=[\"a\",\"b\"]".to_string(),
            ],
            ..RunArgs::default()
        };
        let inputs = coerce_inputs(&args, &typed_flow()).expect("the inputs coerce");
        assert_eq!(
            inputs["count"],
            serde_json::json!(3),
            "a number input coerces from its string"
        );
        assert_eq!(
            inputs["flag"],
            serde_json::json!(true),
            "a boolean input coerces"
        );
        assert_eq!(
            inputs["name"],
            serde_json::json!("release"),
            "a string input passes through"
        );
        assert_eq!(
            inputs["tags"],
            serde_json::json!(["a", "b"]),
            "a k:=<json> value is used as a raw JSON array"
        );
    }

    #[test]
    fn coerce_inputs_rejects_a_value_that_does_not_match_its_declared_type() {
        // Negative space: a non-numeric string for a `number` input is a typed `input_type_mismatch`.
        let args = RunArgs {
            input: vec!["count=lots".to_string()],
            ..RunArgs::default()
        };
        let err = coerce_inputs(&args, &typed_flow()).expect_err("a mistyped input is rejected");
        assert_eq!(err.code, "input_type_mismatch", "the coercion-failure code");
        assert!(
            err.message.contains("count"),
            "the error names the offending input, got {:?}",
            err.message
        );

        // Negative space: a malformed `--input` (no `=`) is a usage error, never a silent drop.
        let malformed = RunArgs {
            input: vec!["justakey".to_string()],
            ..RunArgs::default()
        };
        let err =
            coerce_inputs(&malformed, &typed_flow()).expect_err("a malformed input is rejected");
        assert_eq!(
            err.code, "input_malformed",
            "a bad --input shape is rejected"
        );
    }

    #[test]
    fn apply_env_overrides_sets_context_env_creating_a_context_when_absent() {
        // `--env K=V` overrides land in the context env even when the flow declared no context.
        let flow = resolve_flow(serde_json::json!({ "tasks": [] })).expect("resolves");
        let flow = apply_env_overrides(flow, &["TOKEN=abc".to_string(), "REGION=eu".to_string()])
            .expect("the overrides apply");
        let env = flow
            .context
            .as_ref()
            .and_then(|c| c.env.as_ref())
            .expect("a context env was created");
        assert_eq!(
            env.get("TOKEN").map(String::as_str),
            Some("abc"),
            "the override lands"
        );
        assert_eq!(
            env.get("REGION").map(String::as_str),
            Some("eu"),
            "a second override lands"
        );

        // Negative space: a malformed `--env` (no `=`) is a typed error.
        let flow = resolve_flow(serde_json::json!({ "tasks": [] })).expect("resolves");
        let err =
            apply_env_overrides(flow, &["NOEQUALS".to_string()]).expect_err("malformed --env");
        assert_eq!(err.code, "env_malformed", "a bad --env shape is rejected");
    }

    #[test]
    fn read_state_in_re_validates_and_rejects_a_bad_file() {
        let dir = temp_dir("state-in");
        // A valid object file seeds the state.
        let good = dir.join("good.json");
        std::fs::write(&good, "{\"build\":{\"sha\":\"abc\"}}").expect("write state");
        let seed = read_state_in(good.to_str())
            .expect("a valid state file reads")
            .expect("a seed was produced");
        assert_eq!(
            seed.as_value().get("build"),
            Some(&serde_json::json!({ "sha": "abc" })),
            "the seeded state round-trips"
        );

        // Negative space: a non-object JSON state fails re-validation (`state_not_object`).
        let non_object = dir.join("array.json");
        std::fs::write(&non_object, "[1,2,3]").expect("write array");
        let err = read_state_in(non_object.to_str()).expect_err("a non-object state is rejected");
        assert_eq!(
            err.category,
            ErrorCategory::Validation,
            "a bad seed is a validation error"
        );

        // Negative space: a malformed JSON file is rejected on read, not silently seeded.
        let malformed = dir.join("bad.json");
        std::fs::write(&malformed, "{not json").expect("write malformed");
        let err = read_state_in(malformed.to_str()).expect_err("malformed JSON is rejected");
        assert_eq!(
            err.code, "state_in_invalid",
            "a malformed state file is rejected"
        );

        // Absent `--state-in` seeds nothing.
        assert!(
            read_state_in(None).expect("no state is Ok").is_none(),
            "no seed without --state-in"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_matrix_lowers_two_axes_and_an_authored_map_wins() {
        // Two axes lower to the four-way cross-product, typed values preserved.
        let flow = resolve_flow(serde_json::json!({ "tasks": [] })).expect("resolves");
        let args = RunArgs {
            matrix: vec!["a=1,2".to_string(), "b=x,y".to_string()],
            ..RunArgs::default()
        };
        let combos = resolve_matrix(&args, &flow).expect("the matrix lowers");
        assert_eq!(combos.len(), 4, "2×2 is a four-way cross-product");
        assert_eq!(
            combos[0],
            serde_json::json!({ "a": 1, "b": "x" }),
            "numbers stay numbers, strings strings"
        );

        // An authored `map` wins: `--matrix` is ignored (no combinations), a warning is emitted.
        let mapped = resolve_flow(serde_json::json!({
            "tasks": [ { "name": "fan", "type": "map", "with": {
                "items": ["a"],
                "task": { "type": "exec", "with": { "command": "noop" } }
            } } ]
        }))
        .expect("resolves");
        let combos = resolve_matrix(&args, &mapped).expect("an authored map is not an error");
        assert!(
            combos.is_empty(),
            "an authored map suppresses --matrix (the authored map wins)"
        );
    }

    #[test]
    fn check_concurrency_accepts_within_ceiling_and_rejects_above_it() {
        // A request at or below the ceiling is accepted; unset is accepted; above it is a typed error.
        assert!(
            check_concurrency(None).is_ok(),
            "an unset --concurrency is fine"
        );
        assert!(
            check_concurrency(Some(CONCURRENCY_MAX)).is_ok(),
            "a request at the ceiling is accepted"
        );
        let err = check_concurrency(Some(CONCURRENCY_MAX + 1))
            .expect_err("a request above the ceiling is rejected");
        assert_eq!(err.code, "concurrency_too_high", "the over-ceiling code");
        assert_eq!(
            err.category,
            ErrorCategory::Validation,
            "an over-limit concurrency is a validation error"
        );
    }

    /// A unique temp directory for one test, created under the system temp root.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tmx-run-test-{tag}-{}-{:p}",
            std::process::id(),
            &tag
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn args(flow: Option<&str>, file: Option<&str>) -> RunArgs {
        RunArgs {
            flow: flow.map(str::to_string),
            file: file.map(str::to_string),
            ..RunArgs::default()
        }
    }

    #[test]
    fn resolves_an_explicit_file_and_roots_at_its_directory() {
        let dir = temp_dir("explicit");
        let flow = dir.join("pipeline.yaml");
        std::fs::write(&flow, "tasks: []\n").expect("write flow");

        let resolved = resolve_target(&args(Some("pipeline.yaml"), None), &dir, None)
            .expect("the explicit positional resolves");
        assert!(
            matches!(resolved.target, PreflightTarget::File(_)),
            "an explicit file is a File target"
        );
        assert!(
            resolved.file_reference.is_some(),
            "a file target carries a single reference to drive RunFlow"
        );
        assert_eq!(
            std::fs::canonicalize(&dir).ok().as_deref(),
            Some(resolved.base_dir.as_path()),
            "reference resolution is rooted at the file's own directory"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_flag_takes_precedence_over_positional() {
        let dir = temp_dir("precedence");
        std::fs::write(dir.join("positional.yaml"), "tasks: []\n").expect("write positional");
        std::fs::write(dir.join("explicit.yaml"), "tasks: []\n").expect("write explicit");

        let resolved = resolve_target(
            &args(Some("positional.yaml"), Some("explicit.yaml")),
            &dir,
            Some("env.yaml".to_string()),
        )
        .expect("the --file flag wins");
        let reference = resolved.file_reference.expect("a file reference");
        assert!(
            reference.ends_with("explicit.yaml"),
            "--file beats the positional and $TMX_FLOW, got {reference}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn implicit_search_finds_flow_yaml_then_the_env_fallback() {
        let dir = temp_dir("implicit");
        std::fs::write(dir.join("flow.yaml"), "tasks: []\n").expect("write flow.yaml");

        // No explicit arg: the cwd search finds ./flow.yaml.
        let resolved = resolve_target(&args(None, None), &dir, None).expect("cwd search finds it");
        let reference = resolved.file_reference.expect("a file reference");
        assert!(reference.ends_with("flow.yaml"), "found ./flow.yaml");

        // $TMX_FLOW is consulted before the cwd search: point it at a differently-named file.
        let named = dir.join("other.json");
        std::fs::write(&named, "{\"tasks\":[]}\n").expect("write other.json");
        let resolved = resolve_target(&args(None, None), &dir, Some(named.display().to_string()))
            .expect("the env fallback resolves");
        let reference = resolved.file_reference.expect("a file reference");
        assert!(reference.ends_with("other.json"), "$TMX_FLOW won the order");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_folder_layout_resolves_to_a_directory_target() {
        let dir = temp_dir("layout");
        std::fs::write(dir.join("environment.toml"), "platform = \"local\"\n")
            .expect("write environment");
        std::fs::write(
            dir.join("task-1.yaml"),
            "type: exec\nwith:\n  command: echo hi\n",
        )
        .expect("write task");

        let resolved =
            resolve_target(&args(None, None), &dir, None).expect("the folder layout resolves");
        match resolved.target {
            PreflightTarget::Directory { entries } => {
                assert!(
                    entries.len() >= 2,
                    "the layout's source files are the entries"
                );
            }
            other => panic!("expected a directory target, got {other:?}"),
        }
        assert!(
            resolved.file_reference.is_none(),
            "a directory target has no single file reference"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unresolved_flow_is_a_resolution_error_naming_the_search_path() {
        // Negative space: an empty directory with no explicit arg resolves to nothing — a typed
        // resolution error (CLI exit 4) whose message lists the paths tried.
        let dir = temp_dir("empty");
        let err = resolve_target(&args(None, None), &dir, None)
            .expect_err("an empty cwd resolves no flow");
        assert_eq!(
            err.category,
            ErrorCategory::Resolution,
            "resolution category"
        );
        assert_eq!(err.code, "flow_unresolved", "the unresolved code");
        assert!(
            err.message.contains("flow.yaml"),
            "the message lists the search path, got {:?}",
            err.message
        );

        // An explicitly-named but missing file is also a resolution error naming it.
        let missing = resolve_target(&args(Some("nope.yaml"), None), &dir, None)
            .expect_err("an explicit missing file is an error");
        assert_eq!(
            missing.code, "flow_unresolved",
            "explicit-missing is unresolved"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
