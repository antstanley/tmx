//! The file-path [`ReferenceResolver`] adapter and the cyclic-`flow`-import guard.
//!
//! A `reference` in v0 is a filesystem path, resolved **relative to the referring document's
//! directory** — never the process working directory — so a Flow that imports `../deploy/flow.yaml`
//! means the same thing wherever `tmx` is invoked from
//! ([`.specs/03-loading-and-preflight.md` §Reference resolution](../../../.specs/03-loading-and-preflight.md)).
//!
//! [`FileReferenceResolver`] resolves a single reference. [`assert_acyclic_flow_imports`] is the
//! chain-tracking guard: it walks a Flow's transitive `flow`-type task imports and returns a typed
//! `ResolutionError` the moment an import re-enters an ancestor, rather than recursing forever. (The
//! runtime depth bound `FLOW_DEPTH_MAX` is a second backstop at execution time — see 04.)

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use tmx_core::error::RunError;
use tmx_core::ports::driven::{ReferenceResolver, ResolvedSource, SourceLoader};

use crate::loader::detect_source_kind;

/// A [`ReferenceResolver`] that resolves references as filesystem paths relative to a fixed base
/// directory — the directory of the document that carries the reference.
#[derive(Debug, Clone)]
pub struct FileReferenceResolver {
    base_dir: PathBuf,
}

impl FileReferenceResolver {
    /// A resolver rooted at `base_dir`; references resolve relative to it.
    #[must_use]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// A resolver rooted at the directory *containing* `referrer` — the natural constructor for
    /// "resolve this document's references relative to itself". A referrer with no parent (a bare
    /// filename) resolves against the current directory (`.`).
    #[must_use]
    pub fn for_referrer(referrer: &str) -> Self {
        let parent = Path::new(referrer)
            .parent()
            .filter(|p| !p.as_os_str().is_empty());
        Self::new(parent.map_or_else(|| PathBuf::from("."), Path::to_path_buf))
    }
}

#[async_trait]
impl ReferenceResolver for FileReferenceResolver {
    async fn resolve(&self, reference: &str) -> Result<ResolvedSource, RunError> {
        // Join against the referrer's directory: an absolute reference replaces the base, a relative
        // one extends it — so resolution is anchored to the referrer, not the process cwd.
        let joined = self.base_dir.join(reference);
        let joined_str = joined.to_str().ok_or_else(|| {
            RunError::resolution(
                "non_utf8_path",
                format!("resolved reference path is not valid UTF-8 for `{reference}`"),
            )
        })?;
        // Determine the format from the extension first, so an unknown extension is its own typed
        // error rather than being masked by a not-found from a path we would never load anyway.
        let kind = detect_source_kind(joined_str)?;
        // Canonicalise to confirm the target exists and to get a stable absolute identity (used by
        // cycle detection). A missing/unreadable target is a typed resolution error.
        let canonical = std::fs::canonicalize(&joined).map_err(|e| {
            RunError::resolution(
                "reference_not_found",
                format!("reference `{reference}` did not resolve to a readable file: {e}"),
            )
            .with_path(reference.to_string())
        })?;
        let path = canonical
            .to_str()
            .ok_or_else(|| {
                RunError::resolution(
                    "non_utf8_path",
                    format!("canonical path is not valid UTF-8 for `{reference}`"),
                )
            })?
            .to_string();
        Ok(ResolvedSource { path, kind })
    }
}

/// Walk `entry`'s transitive `flow`-type task imports, returning a typed `ResolutionError`
/// (`code: cyclic_flow_import`) if any import re-enters a document already on the resolution chain.
///
/// Each import is resolved relative to *its own* referrer via [`FileReferenceResolver`], loaded and
/// re-inspected via `loader`, so the guard follows the exact edges preflight will. A diamond (two
/// distinct branches importing the same leaf) is **not** a cycle and does not error; only a back-edge
/// onto an ancestor does.
pub async fn assert_acyclic_flow_imports(
    loader: &dyn SourceLoader,
    entry: &str,
) -> Result<(), RunError> {
    let canonical = std::fs::canonicalize(entry).map_err(|e| {
        RunError::resolution(
            "reference_not_found",
            format!("flow entry `{entry}` did not resolve to a readable file: {e}"),
        )
        .with_path(entry.to_string())
    })?;
    let mut chain: Vec<PathBuf> = Vec::new();
    walk_flow_imports(loader, canonical, &mut chain).await
}

/// Recursive worker for [`assert_acyclic_flow_imports`]: `chain` is the stack of canonical paths from
/// the root to the current node; membership on it is the cycle test.
async fn walk_flow_imports(
    loader: &dyn SourceLoader,
    path: PathBuf,
    chain: &mut Vec<PathBuf>,
) -> Result<(), RunError> {
    if chain.contains(&path) {
        let mut trace: Vec<String> = chain.iter().map(|p| p.display().to_string()).collect();
        trace.push(path.display().to_string());
        return Err(RunError::resolution(
            "cyclic_flow_import",
            format!("cyclic flow import detected: {}", trace.join(" -> ")),
        )
        .with_path(path.display().to_string()));
    }
    let path_str = path.to_str().ok_or_else(|| {
        RunError::resolution(
            "non_utf8_path",
            format!("flow path is not valid UTF-8: {}", path.display()),
        )
    })?;
    let kind = detect_source_kind(path_str)?;
    let value = loader.load(path_str, kind).await?;

    chain.push(path.clone());
    let base_dir = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let resolver = FileReferenceResolver::new(base_dir);
    for reference in flow_import_refs(&value) {
        let resolved = resolver.resolve(&reference).await?;
        let child = PathBuf::from(resolved.path);
        // Box the recursive future: an `async fn` that awaits itself is otherwise infinitely sized.
        Box::pin(walk_flow_imports(loader, child, chain)).await?;
    }
    chain.pop();
    Ok(())
}

/// The `use` references of every `flow`-type task in `value`, across both the array and map task
/// forms. A string shorthand (an `exec` task) and any non-`flow` task contribute nothing.
fn flow_import_refs(value: &Value) -> Vec<String> {
    let mut refs = Vec::new();
    let Some(tasks) = value.get("tasks") else {
        return refs;
    };
    let entries: Vec<&Value> = match tasks {
        Value::Array(array) => array.iter().collect(),
        Value::Object(map) => map.values().collect(),
        _ => return refs,
    };
    for task in entries {
        if task.get("type").and_then(Value::as_str) == Some("flow")
            && let Some(reference) = task
                .get("with")
                .and_then(|with| with.get("use"))
                .and_then(Value::as_str)
        {
            refs.push(reference.to_string());
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flow_import_refs_reads_array_and_map_task_forms() {
        // Array form: only the flow-type task's `use` is an import; the exec task and the flow task
        // without a `use` contribute nothing.
        let array_form = json!({
            "tasks": [
                { "name": "build", "type": "exec", "with": { "command": "make" } },
                { "name": "deploy", "type": "flow", "with": { "use": "../deploy/flow.yaml" } },
                { "name": "noop", "type": "flow", "with": {} }
            ]
        });
        assert_eq!(
            flow_import_refs(&array_form),
            vec!["../deploy/flow.yaml".to_string()],
            "exactly the flow-type task with a `use` is an import"
        );

        // Map form: string shorthands are exec tasks (never imports); a flow-type entry's `use` is.
        let map_form = json!({
            "tasks": {
                "lint": "npm run lint",
                "sub": { "type": "flow", "with": { "use": "./child.yaml" } }
            }
        });
        assert_eq!(
            flow_import_refs(&map_form),
            vec!["./child.yaml".to_string()],
            "the map form's flow entry contributes its `use`, the shorthand does not"
        );
    }

    #[test]
    fn flow_import_refs_of_a_taskless_or_non_object_tasks_is_empty() {
        // Negative space: no `tasks`, or a `tasks` that is not a list/map, yields no imports rather
        // than panicking on the unexpected shape.
        assert!(
            flow_import_refs(&json!({ "name": "x" })).is_empty(),
            "no tasks key"
        );
        assert!(
            flow_import_refs(&json!({ "tasks": "oops" })).is_empty(),
            "a scalar tasks value is not iterated"
        );
    }

    #[test]
    fn for_referrer_roots_at_the_referrer_directory() {
        let resolver = FileReferenceResolver::for_referrer("/a/b/flow.yaml");
        assert_eq!(
            resolver.base_dir,
            PathBuf::from("/a/b"),
            "parent dir is the base"
        );
        // A bare filename has no parent dir, so it anchors at the current directory.
        let bare = FileReferenceResolver::for_referrer("flow.yaml");
        assert_eq!(
            bare.base_dir,
            PathBuf::from("."),
            "a bare referrer anchors at ."
        );
    }
}
