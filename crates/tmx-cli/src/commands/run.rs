//! `tmx run` — the first end-to-end path (07 §`tmx run`).
//!
//! Resolves the Flow reference (the `--file` → positional → `$TMX_FLOW` → `./flow.{…}`/`./tmx.{…}` →
//! folder-layout order), preflights it (fail-fast validation + the capability check, 03 §Preflight
//! flow), then executes it and returns the terminal [`RunRecord`]. A single file runs through the
//! [`RunFlow`] use case (the reference-driven load → resolve → run → mask pipeline); a directory /
//! folder layout — which has no single file reference — runs the assembled, preflighted Flow directly
//! through the [`PipelineRunner`]. Either way the final Pipeline state comes back masked, and `main`
//! renders it to stdout and maps the outcome to an exit code.

use std::path::{Path, PathBuf};

use tmx_adapters::loader::detect_source_kind;

use tmx_core::ports::driven::IdGenerator;
use tmx_core::ports::driving::{RunFlow, RunOptions};
use tmx_core::{
    Masker, Milliseconds, PipelineState, PreflightTarget, Preflighted, ResolvedFlow, RunConfig,
    RunError, RunRecord, merged_inputs, preflight,
};

use crate::args::RunArgs;
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
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let resolved = resolve_target(&args, &cwd, config::env_flow())?;

    let composed = Composed::new(resolved.base_dir.clone())?;
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

    let config = RunConfig::default();
    match &resolved.file_reference {
        // A single file runs through the RunFlow use case (the reference-driven pipeline).
        Some(reference) => {
            let use_case = composed.run_flow(config);
            use_case
                .run(reference, serde_json::json!({}), RunOptions::default())
                .await
        }
        // A directory / folder layout has no single file reference; run the assembled, preflighted
        // Flow directly. This mirrors the RunFlow use case's own tail (mint id → run → mask final
        // state → build the record) for the target the reference-driven use case cannot express.
        None => execute_preflighted(&preflighted, &composed, config).await,
    }
}

/// Execute an already-preflighted, assembled [`ResolvedFlow`] directly through the runner, returning
/// the masked terminal [`RunRecord`] — the directory / folder-layout path.
async fn execute_preflighted(
    preflighted: &Preflighted,
    composed: &Composed,
    config: RunConfig,
) -> Result<RunRecord, RunError> {
    let flow: &ResolvedFlow = &preflighted.flow;
    let ports = composed.ports();
    let merged = merged_inputs(&serde_json::json!({}), &flow.inputs);

    let id = composed.ids().new_run_id();
    let started_at = ports.clock.now();
    let start_ms = ports.clock.now_ms();
    let mut masker = Masker::new();
    let mut resolved_secrets: Vec<String> = Vec::new();

    let pipeline = composed
        .runner(config)
        .run(
            &id,
            flow,
            &merged,
            ports,
            &mut masker,
            &mut resolved_secrets,
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
    use tmx_core::ErrorCategory;

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
