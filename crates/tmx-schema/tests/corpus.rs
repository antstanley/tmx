//! Corpus round-trip: every artifact under `docs/examples/` deserialises into the task-03 input
//! model without loss, and both the array form and the map form of a task list preserve source
//! order.
//!
//! This is the reviewable evidence for task 03 (schema input model). The example corpus is the
//! frozen `tmx.schema.json` contract expressed as concrete YAML/JSON/JSONC/TOML documents; if a
//! `$def` were missing a Rust type, or a field were mis-modelled, the matching artifact would fail
//! to deserialise here. The multi-format parsing in this test is DEV-ONLY scaffolding — the real
//! cross-format `SourceLoader` is task 13; here the parsers exist only to feed the corpus through
//! these types.
//!
//! Order preservation is proven twice: against the corpus (an alphabetically-keyed map is a weak
//! witness, since a sorted map would coincide), and against a synthetic map whose keys are NOT in
//! sorted order — the latter fails outright under a `BTreeMap`/`HashMap`, so it pins the map form
//! to `indexmap::IndexMap`.

use std::path::{Path, PathBuf};

use tmx_schema::flow::Tasks;
use tmx_schema::{Context, Environment, Flow, Task};

/// The `docs/examples/` directory, resolved from this crate's manifest dir
/// (`<root>/crates/tmx-schema`) so the test is independent of the process working directory.
fn examples_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = match manifest_dir.parent().and_then(Path::parent) {
        Some(root) => root.to_path_buf(),
        None => panic!(
            "workspace root is two levels above {}",
            manifest_dir.display()
        ),
    };
    root.join("docs").join("examples")
}

/// Strip `//` line comments and `/* … */` block comments from JSONC, leaving string literals
/// intact — the corpus's `.jsonc` files carry `//`-comments AND string values containing `//`
/// (e.g. `"https://…"`), so the stripper must track whether it is inside a double-quoted string.
fn strip_jsonc_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
            out.push('"');
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            // Line comment: consume to end of line (keep the newline for line-accurate errors).
            chars.next();
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            // Block comment: consume to the closing `*/`.
            chars.next();
            let mut prev = '\0';
            for next in chars.by_ref() {
                if prev == '*' && next == '/' {
                    break;
                }
                prev = next;
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Deserialise `text` (in the format implied by `ext`) into `T`, panicking with the file path on
/// failure so a corpus regression names the offending artifact.
fn parse<T: serde::de::DeserializeOwned>(path: &Path, ext: &str, text: &str) -> T {
    let result: Result<T, String> = match ext {
        "json" => serde_json::from_str(text).map_err(|e| e.to_string()),
        "jsonc" => serde_json::from_str(&strip_jsonc_comments(text)).map_err(|e| e.to_string()),
        "yaml" | "yml" => serde_yaml_ng::from_str(text).map_err(|e| e.to_string()),
        "toml" => toml::from_str(text).map_err(|e| e.to_string()),
        other => panic!("unhandled extension .{other} for {}", path.display()),
    };
    match result {
        Ok(value) => value,
        Err(err) => panic!("failed to deserialise {}: {err}", path.display()),
    }
}

/// The artifact kind of a corpus file — its `kind` discriminator (present on almost every artifact)
/// or, absent that, "a top-level document is a Flow".
#[derive(serde::Deserialize)]
struct KindPeek {
    kind: Option<String>,
}

/// Recursively collect every regular file under `dir`, sorted for a deterministic order.
fn collect_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", current.display()));
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(e) => panic!(
                    "cannot read a directory entry in {}: {e}",
                    current.display()
                ),
            };
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn every_corpus_artifact_deserialises() {
    let dir = examples_dir();
    let files = collect_files(&dir);
    assert!(
        files.len() >= 20,
        "the example corpus should have many artifacts, found {}",
        files.len()
    );

    let mut flows = 0_usize;
    let mut contexts = 0_usize;
    let mut environments = 0_usize;
    let mut tasks = 0_usize;
    let mut skipped = 0_usize;

    for path in &files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        // The corpus also carries a README and a provider manifest; neither is one of the four
        // artifact kinds task 03 mirrors, so they are outside this test's scope.
        if !matches!(ext.as_str(), "json" | "jsonc" | "yaml" | "yml" | "toml") {
            skipped += 1;
            continue;
        }
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let peek: KindPeek = parse(path, &ext, &text);
        match peek.kind.as_deref() {
            Some("flow") | None => {
                let _flow: Flow = parse(path, &ext, &text);
                flows += 1;
            }
            Some("context") => {
                let _ctx: Context = parse(path, &ext, &text);
                contexts += 1;
            }
            Some("environment") => {
                let _env: Environment = parse(path, &ext, &text);
                environments += 1;
            }
            Some("task") => {
                let _task: Task = parse(path, &ext, &text);
                tasks += 1;
            }
            // e.g. the `provider` manifest — not modelled by task 03.
            Some(_) => {
                skipped += 1;
            }
        }
    }

    // Every mirrored artifact kind appears at least once, so the four-way dispatch is exercised.
    assert!(flows >= 8, "expected several Flow artifacts, found {flows}");
    assert!(
        contexts >= 2,
        "expected standalone Contexts, found {contexts}"
    );
    assert!(
        environments >= 2,
        "expected standalone Environments, found {environments}"
    );
    assert!(tasks >= 3, "expected standalone Tasks, found {tasks}");
    assert!(skipped >= 1, "expected the provider/README to be skipped");
}

#[test]
fn array_form_flow_preserves_source_order() {
    let dir = examples_dir();
    let path = dir.join("single-file-flow.json");
    let text = std::fs::read_to_string(&path).expect("read single-file-flow.json");
    let flow: Flow = parse(&path, "json", &text);

    let Tasks::List(_) = &flow.tasks else {
        panic!("single-file-flow.json must deserialise into the array (List) task form");
    };
    let names: Vec<&str> = flow
        .tasks
        .names_in_order()
        .into_iter()
        .map(|n| n.expect("every array-form task in this Flow has a name"))
        .collect();
    assert_eq!(
        names,
        ["test", "build", "upload", "summarize", "verify", "deploy"],
        "array-form tasks must emerge in document order"
    );
    assert_eq!(flow.tasks.len(), 6, "the Flow has six tasks");
}

#[test]
fn map_form_flow_preserves_source_order() {
    let dir = examples_dir();
    let path = dir.join("map-tasks.yaml");
    let text = std::fs::read_to_string(&path).expect("read map-tasks.yaml");
    let flow: Flow = parse(&path, "yaml", &text);

    let Tasks::Map(map) = &flow.tasks else {
        panic!("map-tasks.yaml must deserialise into the name-keyed (Map) task form");
    };
    let names: Vec<&str> = map.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["build", "lint", "test"],
        "map-form tasks must emerge in document key order"
    );
    // The `lint` value is the string shorthand for an exec task; the others are full objects.
    assert!(
        matches!(map.get("lint"), Some(tmx_schema::TaskEntry::Shorthand(_))),
        "the `lint` entry is an exec string shorthand"
    );
    assert!(
        matches!(map.get("build"), Some(tmx_schema::TaskEntry::Task(_))),
        "the `build` entry is a full task object"
    );
}

#[test]
fn map_form_preserves_unsorted_order_pinning_indexmap() {
    // A map whose keys are NOT in sorted order: `zebra` < `alpha` < `mango` would be reordered to
    // `alpha, mango, zebra` by a BTreeMap and scrambled by a HashMap. Only an insertion-ordered
    // IndexMap yields the source order, so this is the load-bearing witness that the map form is
    // order-preserving as a type property (certificate O2 check).
    let doc = r#"{
      "tasks": {
        "zebra": "echo zebra",
        "alpha": "echo alpha",
        "mango": "echo mango"
      }
    }"#;
    let flow: Flow = serde_json::from_str(doc).expect("synthetic map-form flow deserialises");
    let Tasks::Map(map) = &flow.tasks else {
        panic!("a JSON object task collection must be the Map form");
    };
    let names: Vec<&str> = map.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["zebra", "alpha", "mango"],
        "unsorted source key order must survive — proves IndexMap, not a sorted/hashed map"
    );
    assert_ne!(
        names,
        ["alpha", "mango", "zebra"],
        "a BTreeMap would have sorted these keys; the map form must not"
    );
}

#[test]
fn a_mismatched_with_and_type_fails_to_deserialise() {
    // Negative space: the `with` payload must match its `type` discriminant. An `exec` task whose
    // `with` is a `fetch` payload has no `command` (required) and an unknown `url`, so the adjacent
    // ExecWith deserialisation fails rather than silently accepting the wrong shape.
    let exec_with_fetch_payload =
        r#"{ "name": "x", "type": "exec", "with": { "url": "https://example.com" } }"#;
    assert!(
        serde_json::from_str::<Task>(exec_with_fetch_payload).is_err(),
        "an exec task carrying a fetch payload must not deserialise"
    );

    // The mirror image: a `fetch` task whose `with` is an `exec` payload lacks the required `url`.
    let fetch_with_exec_payload = r#"{ "type": "fetch", "with": { "command": "npm test" } }"#;
    assert!(
        serde_json::from_str::<Task>(fetch_with_exec_payload).is_err(),
        "a fetch task carrying an exec payload must not deserialise"
    );

    // A task type outside the closed set of ten is rejected by the adjacently-tagged enum.
    let unknown_type = r#"{ "type": "teleport", "with": {} }"#;
    assert!(
        serde_json::from_str::<Task>(unknown_type).is_err(),
        "an unknown task type must not deserialise"
    );

    // A known type with its required `with` omitted fails: every variant carries a payload.
    let missing_with = r#"{ "type": "exec" }"#;
    assert!(
        serde_json::from_str::<Task>(missing_with).is_err(),
        "a task type with no `with` payload must not deserialise"
    );
}
