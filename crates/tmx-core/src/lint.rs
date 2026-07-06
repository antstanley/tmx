//! The static **lint** dataflow pass — the deeper-than-schema analysis behind `tmx lint`
//! ([03 §`lint`](../../../.specs/03-loading-and-preflight.md)).
//!
//! [`analyze_flow`] walks a loaded Flow value and emits a [`Diagnostic`] for each dataflow defect the
//! schema cannot express: a `${{ tasks.NAME.field }}` read whose `field` is not a declared property of
//! `NAME`'s `produces` schema (catching a typo like `tasks.build.artifcat`), an `inputs.NAME` read of
//! an undeclared input, a `secrets.NAME` read the task did not list in its `secrets`, and a duplicate
//! or missing array-form task `name`. Every finding is a **warning**: a clean `tmx lint` prints them
//! and exits `0`, while `--strict` promotes each to an exit-`3` error (the CLI owns that promotion).
//!
//! The pass is **pure** — it takes the loaded `serde_json::Value` and returns diagnostics, reaching no
//! port. Reference resolution and cyclic-`flow`-import detection (which do reach the loader/resolver
//! ports) live in the [`LintFlow`](crate::ports::driving::LintFlow) use case that calls this pass; this
//! module is the sync dataflow half.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::model::{Diagnostic, Severity};

/// One namespace-rooted reference statically extracted from a `${{ … }}` expression.
///
/// Only the three roots the dataflow pass checks are captured — `inputs.*`, `secrets.*`, and
/// `tasks.NAME.field`; every other namespace (`env`, `item`, `matrix`, …) is ignored here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ref {
    /// `inputs.NAME` — a declared-input read.
    Input(String),
    /// `secrets.NAME` — a task-listed-secret read.
    Secret(String),
    /// `tasks.NAME.FIELD` — a prior task's output read; `field` is `None` for a bare `tasks.NAME`.
    Task {
        /// The referenced task name.
        name: String,
        /// The first field read off that task's output, when the expression names one.
        field: Option<String>,
    },
}

/// Analyse a loaded Flow `value` for dataflow defects, appending a warning [`Diagnostic`] per finding.
///
/// The pass is purely structural over the loaded JSON: it never resolves a reference or reaches a port
/// (that is the use case's job). It checks four families of defect — undeclared inputs, unlisted
/// secrets, typo'd `produces` reads, and duplicate/missing task names — each surfaced as a warning so
/// the CLI can decide (`--strict`) whether to fail on it.
pub(crate) fn analyze_flow(value: &Value, diagnostics: &mut Vec<Diagnostic>) {
    let declared_inputs = declared_input_names(value);
    let tasks = ordered_tasks(value);

    // The set of declared task names, and the per-name `produces` schema (when a task declares one).
    let task_names: BTreeSet<&str> = tasks
        .iter()
        .filter_map(|t| t.name.as_deref())
        .filter(|n| !n.is_empty())
        .collect();

    check_task_names(&tasks, diagnostics);

    for task in &tasks {
        // The secrets a task (and any nested inner task) lists — the allowed set for `secrets.*` reads
        // anywhere in this task's subtree.
        let mut allowed_secrets: BTreeSet<String> = BTreeSet::new();
        collect_secret_names(task.raw, &mut allowed_secrets);

        let mut expressions: Vec<String> = Vec::new();
        collect_expressions(task.raw, &mut expressions);

        let where_task = task.name.clone().unwrap_or_default();
        for expression in &expressions {
            for reference in extract_refs(expression) {
                check_reference(
                    &reference,
                    &declared_inputs,
                    &task_names,
                    &tasks,
                    &allowed_secrets,
                    &where_task,
                    diagnostics,
                );
            }
        }
    }
}

/// A single task as the pass sees it: its declared name (map key or `name` field) and its raw JSON.
struct TaskView<'a> {
    name: Option<String>,
    raw: &'a Value,
}

/// The declared Flow input names — the keys of the top-level `inputs` object (empty when absent).
fn declared_input_names(value: &Value) -> BTreeSet<String> {
    value
        .get("inputs")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// The Flow's tasks in source order, as [`TaskView`]s — the array form yields each element's `name`
/// field, the map form yields its key (a string-shorthand value keeps its key as the name).
fn ordered_tasks(value: &Value) -> Vec<TaskView<'_>> {
    match value.get("tasks") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|raw| TaskView {
                name: raw.get("name").and_then(Value::as_str).map(str::to_string),
                raw,
            })
            .collect(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(name, raw)| TaskView {
                name: Some(name.clone()),
                raw,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Flag a missing (array-form) or duplicate task `name` — the same structural checks preflight
/// enforces as hard errors, surfaced statically as warnings.
fn check_task_names(tasks: &[TaskView<'_>], diagnostics: &mut Vec<Diagnostic>) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for task in tasks {
        match task.name.as_deref() {
            None | Some("") => diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "missing_task_name",
                "an array-form task is missing a non-empty `name`",
            )),
            Some(name) => {
                if !seen.insert(name) {
                    diagnostics.push(
                        Diagnostic::new(
                            Severity::Warning,
                            "duplicate_task_name",
                            format!("duplicate task name `{name}`"),
                        )
                        .with_path(name.to_string()),
                    );
                }
            }
        }
    }
}

/// Check one extracted [`Ref`] against the declared inputs, task names + `produces` schemas, and the
/// task's allowed secrets, appending a warning when the reference cannot be satisfied statically.
fn check_reference(
    reference: &Ref,
    declared_inputs: &BTreeSet<String>,
    task_names: &BTreeSet<&str>,
    tasks: &[TaskView<'_>],
    allowed_secrets: &BTreeSet<String>,
    where_task: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match reference {
        Ref::Input(name) => {
            if !declared_inputs.contains(name) {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "undeclared_input",
                        format!(
                            "task `{where_task}` reads `inputs.{name}`, but no input `{name}` is declared"
                        ),
                    )
                    .with_path(format!("tasks.{where_task}")),
                );
            }
        }
        Ref::Secret(name) => {
            if !allowed_secrets.contains(name) {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "undeclared_secret",
                        format!(
                            "task `{where_task}` reads `secrets.{name}`, but `{name}` is not listed in its `secrets`"
                        ),
                    )
                    .with_path(format!("tasks.{where_task}")),
                );
            }
        }
        Ref::Task { name, field } => {
            if !task_names.contains(name.as_str()) {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "unknown_task_reference",
                        format!(
                            "task `{where_task}` reads `tasks.{name}`, but no task `{name}` is defined"
                        ),
                    )
                    .with_path(format!("tasks.{where_task}")),
                );
                return;
            }
            // The typo check: a `tasks.NAME.field` read whose `field` is not a declared property of
            // NAME's `produces` schema (only checkable when that task declares an object `produces`).
            if let Some(field) = field
                && let Some(produces) = produces_schema(tasks, name)
                && produces_omits_field(produces, field)
            {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Warning,
                        "produces_field_unknown",
                        format!(
                            "task `{where_task}` reads `tasks.{name}.{field}`, but `{field}` is not a property of task `{name}`'s `produces` schema"
                        ),
                    )
                    .with_path(format!("tasks.{name}.produces")),
                );
            }
        }
    }
}

/// The `produces` schema declared by the top-level task `name`, when it has one.
fn produces_schema<'a>(tasks: &'a [TaskView<'a>], name: &str) -> Option<&'a Value> {
    tasks
        .iter()
        .find(|t| t.name.as_deref() == Some(name))
        .and_then(|t| t.raw.get("produces"))
}

/// Whether `produces` declares an object `properties` map that does **not** contain `field` — the
/// static signal of a typo'd read. Returns `false` when the schema has no `properties` map to check
/// against (a non-object `produces`, or one without declared properties, cannot rule the field out).
fn produces_omits_field(produces: &Value, field: &str) -> bool {
    produces
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| !props.contains_key(field))
}

/// Recursively collect every `secrets` **string array** in `value` — a task's listed secret names,
/// including any nested inner task's. A `context.secrets` map (an object, not an array) is skipped, so
/// only true secret-name lists are gathered.
fn collect_secret_names(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(list)) = map.get("secrets") {
                for item in list {
                    if let Some(name) = item.as_str() {
                        out.insert(name.to_string());
                    }
                }
            }
            for child in map.values() {
                collect_secret_names(child, out);
            }
        }
        Value::Array(list) => {
            for item in list {
                collect_secret_names(item, out);
            }
        }
        _ => {}
    }
}

/// Recursively collect every `${{ … }}` inner expression from the string leaves of `value`.
fn collect_expressions(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => extract_interpolations(text, out),
        Value::Array(list) => {
            for item in list {
                collect_expressions(item, out);
            }
        }
        Value::Object(map) => {
            for child in map.values() {
                collect_expressions(child, out);
            }
        }
        _ => {}
    }
}

/// Append every `${{ expr }}` inner expression found in `text` (trimmed) to `out`. An unterminated
/// `${{` at the end is ignored — the runtime interpolator reports it; the lint pass only harvests the
/// well-formed reads it can analyse.
fn extract_interpolations(text: &str, out: &mut Vec<String>) {
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        let after = &rest[start + 3..];
        let Some(end) = after.find("}}") else {
            break;
        };
        out.push(after[..end].trim().to_string());
        rest = &after[end + 2..];
    }
}

/// Statically extract the `inputs.*` / `secrets.*` / `tasks.NAME.field` references from one `${{ }}`
/// inner `expression`.
///
/// A hand-written scanner, not the full interpolation parser: it finds each namespace **root**
/// identifier (one not itself a member access — i.e. not immediately preceded by `.`), reads the
/// dotted field(s) after it, and ignores string-literal contents so a quoted `inputs.x` is not
/// mistaken for a read. Bracketed indexing (`tasks["a"].b`) is deliberately not analysed — it yields
/// no reference rather than a false positive.
fn extract_refs(expression: &str) -> Vec<Ref> {
    let bytes = expression.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0usize;
    // The last non-whitespace byte seen before the current token — `.` marks a member access, so an
    // identifier preceded by `.` is a field, never a namespace root.
    let mut prev: u8 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'"' || c == b'\'' {
            i = skip_string(bytes, i);
            prev = c;
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            if prev != b'.' {
                let ident = &expression[start..i];
                if let Some(reference) = read_root(ident, bytes, expression, i) {
                    refs.push(reference);
                }
            }
            prev = b'a'; // a non-`.` significant byte
            continue;
        }
        if !c.is_ascii_whitespace() {
            prev = c;
        }
        i += 1;
    }
    refs
}

/// Interpret a root `ident` at byte offset `after` (just past the identifier) as a namespace read,
/// peeking the dotted field(s) that follow. Returns `None` for a non-root identifier or a bare root
/// with no field.
fn read_root(ident: &str, bytes: &[u8], expression: &str, after: usize) -> Option<Ref> {
    match ident {
        "inputs" => read_field(bytes, expression, after).0.map(Ref::Input),
        "secrets" => read_field(bytes, expression, after).0.map(Ref::Secret),
        "tasks" => {
            let (name, next) = read_field(bytes, expression, after);
            let name = name?;
            let (field, _) = read_field(bytes, expression, next);
            Some(Ref::Task { name, field })
        }
        _ => None,
    }
}

/// Peek a `.identifier` step starting at `pos` (skipping spaces): returns the field name and the byte
/// offset just past it, or `(None, pos)` when the next token is not a dotted field.
fn read_field(bytes: &[u8], expression: &str, pos: usize) -> (Option<String>, usize) {
    let mut i = skip_spaces(bytes, pos);
    if i >= bytes.len() || bytes[i] != b'.' {
        return (None, pos);
    }
    i = skip_spaces(bytes, i + 1);
    if i >= bytes.len() || !is_ident_start(bytes[i]) {
        return (None, pos);
    }
    let start = i;
    i += 1;
    while i < bytes.len() && is_ident_continue(bytes[i]) {
        i += 1;
    }
    (Some(expression[start..i].to_string()), i)
}

/// The byte offset of the first non-space byte at or after `pos`.
fn skip_spaces(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    pos
}

/// Skip a quoted string literal that opens at `start`, honouring backslash escapes; returns the byte
/// offset just past the closing quote (or the string end when unterminated).
fn skip_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Whether `b` may start an identifier (ASCII letter or `_`).
fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Whether `b` may continue an identifier (letter, digit, or `_`).
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.code).collect()
    }

    #[test]
    fn extract_refs_reads_roots_and_ignores_members_and_strings() {
        // A `tasks.NAME.field` read yields the name and the first field; the second field is the
        // produces-checked one.
        let refs = extract_refs("tasks.build.artifcat");
        assert_eq!(
            refs,
            vec![Ref::Task {
                name: "build".to_string(),
                field: Some("artifcat".to_string()),
            }],
            "tasks.NAME.field extracts the name and field"
        );

        // `inputs.x` and `secrets.y` extract their names; an identifier inside a string literal or a
        // member access (`out.inputs.z`) is NOT a root read.
        let refs = extract_refs("inputs.name === \"inputs.fake\" && out.inputs.z || secrets.TOKEN");
        assert_eq!(
            refs,
            vec![
                Ref::Input("name".to_string()),
                Ref::Secret("TOKEN".to_string()),
            ],
            "roots are read, string contents and member fields are not: {refs:?}"
        );

        // Bracketed indexing yields no reference (not a false positive), and a bare root none.
        assert!(
            extract_refs("tasks[\"build\"].out").is_empty(),
            "bracket form is not analysed"
        );
        assert!(
            extract_refs("inputs === inputs").is_empty(),
            "a bare root with no field yields nothing"
        );
    }

    #[test]
    fn analyze_flags_typo_undeclared_input_and_unlisted_secret() {
        // A Flow whose second task reads a typo'd produces field, an undeclared input, and an unlisted
        // secret — each is a distinct warning; the correct reads on the first task are silent.
        let flow = json!({
            "inputs": { "name": { "type": "string" } },
            "tasks": [
                {
                    "name": "build",
                    "type": "exec",
                    "with": { "command": "echo ${{ inputs.name }}" },
                    "produces": {
                        "type": "object",
                        "properties": { "artifact": { "type": "string" } }
                    }
                },
                {
                    "name": "ship",
                    "type": "exec",
                    "secrets": ["TOKEN"],
                    "with": {
                        "command": "deploy ${{ tasks.build.artifcat }} ${{ inputs.missing }} ${{ secrets.OTHER }} ${{ secrets.TOKEN }}"
                    }
                }
            ]
        });
        let mut diagnostics = Vec::new();
        analyze_flow(&flow, &mut diagnostics);
        let found = codes(&diagnostics);
        assert!(
            found.contains(&"produces_field_unknown"),
            "the typo'd produces read is flagged: {found:?}"
        );
        assert!(
            found.contains(&"undeclared_input"),
            "the undeclared input is flagged: {found:?}"
        );
        assert!(
            found.contains(&"undeclared_secret"),
            "the unlisted secret is flagged: {found:?}"
        );
        // Negative space: the correctly-declared reads (inputs.name, secrets.TOKEN, and a correct
        // produces read) draw NO diagnostic — exactly three findings, one per seeded defect.
        assert_eq!(
            diagnostics.len(),
            3,
            "only the three seeded defects are flagged, got {found:?}"
        );
    }

    #[test]
    fn analyze_is_clean_on_a_correct_flow() {
        // The negative-space companion: a Flow whose every read is declared draws no diagnostic at all.
        let flow = json!({
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
        let mut diagnostics = Vec::new();
        analyze_flow(&flow, &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "a fully-declared flow lints clean, got {:?}",
            codes(&diagnostics)
        );
    }

    #[test]
    fn analyze_flags_duplicate_and_missing_task_names() {
        // Array-form duplicate and missing names are surfaced statically.
        let flow = json!({
            "tasks": [
                { "name": "a", "type": "exec", "with": { "command": "x" } },
                { "name": "a", "type": "exec", "with": { "command": "y" } },
                { "type": "exec", "with": { "command": "z" } }
            ]
        });
        let mut diagnostics = Vec::new();
        analyze_flow(&flow, &mut diagnostics);
        let found = codes(&diagnostics);
        assert!(
            found.contains(&"duplicate_task_name"),
            "the duplicate name is flagged: {found:?}"
        );
        assert!(
            found.contains(&"missing_task_name"),
            "the nameless task is flagged: {found:?}"
        );
    }

    #[test]
    fn map_form_shorthand_reads_are_analysed_under_their_key() {
        // A map-form string shorthand keeps its key as the task name and its command is still scanned
        // for interpolation reads — an undeclared input inside a shorthand is caught.
        let flow = json!({
            "tasks": {
                "greet": "echo ${{ inputs.missing }}"
            }
        });
        let mut diagnostics = Vec::new();
        analyze_flow(&flow, &mut diagnostics);
        assert_eq!(
            codes(&diagnostics),
            vec!["undeclared_input"],
            "the shorthand's undeclared input is flagged"
        );
    }

    #[test]
    fn unknown_task_reference_is_flagged_and_short_circuits_the_field_check() {
        // A read of a task that does not exist is its own diagnostic; no produces-field check follows.
        let flow = json!({
            "tasks": [
                { "name": "only", "type": "exec", "with": { "command": "echo ${{ tasks.ghost.x }}" } }
            ]
        });
        let mut diagnostics = Vec::new();
        analyze_flow(&flow, &mut diagnostics);
        assert_eq!(
            codes(&diagnostics),
            vec!["unknown_task_reference"],
            "an unknown task reference is flagged exactly once"
        );
    }
}
