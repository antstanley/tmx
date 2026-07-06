// This whole crate is test code: an `expect`/`unwrap` here IS the assertion and its panic IS the
// failure signal. clippy's `allow-*-in-tests` only covers `#[test]`/`#[cfg(test)]` items, not an
// integration-test crate's free helpers, so the workspace-denied lints are re-permitted here.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end tests for the run store and `tmx runs` (task 27 definition of done).
//!
//! These drive the *real* compiled `tmx` binary over temp-dir fixtures, exercising the whole
//! composition — a run persisting to `./.tmx/runs/<uuidv7>/`, the storing event tee, and the
//! `tmx runs list/show/state/logs/prune/rm` queries — as a reviewer running it from the shell would.
//! They are the O1/O2/O4 obligations made executable:
//!
//! - a run persists a record and event log; `tmx runs list` shows runs chronologically; `state`/`logs`
//!   dump the masked snapshot and replayed log; `--no-store` records nothing;
//! - a persisted event log replays with its secrets already masked (persist-after-mask);
//! - `tmx runs prune` removes an aged record while keeping a fresh one.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// A unique temp directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tmx-cli-runs-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write `contents` to `dir/name` and return its path.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).expect("write fixture");
    path
}

/// The captured result of running the binary.
struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run `tmx <args…>` from `dir`, capturing stdout/stderr and the exit code.
fn run_tmx(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmx"));
    command.current_dir(dir).args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("the tmx binary runs");
    Output {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

const PASSING_YAML: &str = "name: demo\ntasks:\n  - name: build\n    type: exec\n    with:\n      command: printf built-ok\n";

/// Parse the `runs` array of ids from a `tmx runs list` stdout.
fn listed_ids(stdout: &str) -> Vec<String> {
    let value: Value =
        serde_json::from_str(stdout.trim()).expect("`tmx runs list` prints one JSON object");
    value["runs"]
        .as_array()
        .expect("a runs array")
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .expect("each run has a string id")
                .to_string()
        })
        .collect()
}

#[test]
fn a_run_persists_and_runs_list_show_state_logs_dump_it() {
    // O1/O4: a run persists to ./.tmx/runs/<id>/; `tmx runs list` shows it; show/state/logs dump the
    // masked snapshot and replayed log.
    let dir = temp_dir("persist");
    write(&dir, "flow.yaml", PASSING_YAML);

    let run = run_tmx(&dir, &["run", "flow.yaml"], &[]);
    assert_eq!(run.code, Some(0), "the run exits 0; stderr: {}", run.stderr);

    // The record and log landed on disk under a single UUIDv7 directory.
    let runs_dir = dir.join(".tmx").join("runs");
    let entries: Vec<_> = std::fs::read_dir(&runs_dir)
        .expect("the run store directory exists")
        .flatten()
        .collect();
    assert_eq!(entries.len(), 1, "exactly one run persisted");
    let run_dir = entries[0].path();
    assert!(run_dir.join("record.json").is_file(), "a record snapshot");
    assert!(run_dir.join("log.ndjson").is_file(), "an ndjson event log");

    // `tmx runs list` shows the run.
    let list = run_tmx(&dir, &["runs", "list"], &[]);
    assert_eq!(list.code, Some(0), "runs list exits 0; {}", list.stderr);
    let ids = listed_ids(&list.stdout);
    assert_eq!(ids.len(), 1, "one run is listed");
    let id = &ids[0];

    // `show` dumps the full record; `state` the final state; `logs` the replayed event stream.
    let show = run_tmx(&dir, &["runs", "show", id], &[]);
    assert_eq!(show.code, Some(0), "runs show exits 0; {}", show.stderr);
    let record: Value = serde_json::from_str(show.stdout.trim()).expect("show prints the record");
    assert_eq!(record["id"], id.as_str(), "show carries the id");
    assert_eq!(record["status"], "ok", "the run completed ok");

    let state = run_tmx(&dir, &["runs", "state", id], &[]);
    let final_state: Value =
        serde_json::from_str(state.stdout.trim()).expect("state prints the final state");
    assert_eq!(
        final_state["build"]["message"], "built-ok",
        "state dumps the merged final state"
    );

    let logs = run_tmx(&dir, &["runs", "logs", id], &[]);
    let replay: Value = serde_json::from_str(logs.stdout.trim()).expect("logs prints the events");
    let events = replay["events"].as_array().expect("an events array");
    assert_eq!(
        events.first().map(|e| &e["event"]),
        Some(&Value::String("run.start".to_string())),
        "the replayed log opens with run.start"
    );
    assert_eq!(
        events.last().map(|e| &e["event"]),
        Some(&Value::String("run.finish".to_string())),
        "the replayed log closes with run.finish"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn runs_list_is_chronological_across_two_runs() {
    // O1/residue: two runs list in chronological (UUIDv7) order — the earlier run first.
    let dir = temp_dir("chrono");
    write(&dir, "flow.yaml", PASSING_YAML);
    assert_eq!(run_tmx(&dir, &["run", "flow.yaml"], &[]).code, Some(0));
    assert_eq!(run_tmx(&dir, &["run", "flow.yaml"], &[]).code, Some(0));

    let list = run_tmx(&dir, &["runs", "list"], &[]);
    let ids = listed_ids(&list.stdout);
    assert_eq!(ids.len(), 2, "both runs are listed");
    assert!(
        ids[0] < ids[1],
        "runs list chronologically by id: {} before {}",
        ids[0],
        ids[1]
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_store_records_nothing() {
    // O1 negative space: `tmx run --no-store` writes nothing under ./.tmx/runs/.
    let dir = temp_dir("nostore");
    write(&dir, "flow.yaml", PASSING_YAML);

    let run = run_tmx(&dir, &["run", "flow.yaml", "--no-store"], &[]);
    assert_eq!(run.code, Some(0), "the run exits 0; {}", run.stderr);
    assert!(
        !run.stdout.trim().is_empty(),
        "the run still prints its final state to stdout"
    );

    let runs_dir = dir.join(".tmx").join("runs");
    let empty = !runs_dir.exists()
        || std::fs::read_dir(&runs_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(true);
    assert!(empty, "--no-store leaves the run store empty");

    // `tmx runs list` over an empty store is an empty array, not an error.
    let list = run_tmx(&dir, &["runs", "list"], &[]);
    assert_eq!(list.code, Some(0), "runs list exits 0 on an empty store");
    assert!(listed_ids(&list.stdout).is_empty(), "no runs are listed");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_persisted_log_replays_with_secrets_masked() {
    // O1 check / P3 invariant: a secret a task echoes is already masked in the persisted log — a replay
    // through `tmx runs logs` cannot re-expose it (persist-after-mask).
    let dir = temp_dir("masked");
    let secret = "supersecretpersistedvalue";
    write(
        &dir,
        "flow.yaml",
        "name: demo\ncontext:\n  secrets:\n    TOKEN:\n      env: TMX_TEST_TOKEN\ntasks:\n  - name: leak\n    type: exec\n    secrets: [TOKEN]\n    with:\n      command: \"printf %s '${{ secrets.TOKEN }}'\"\n",
    );
    let run = run_tmx(&dir, &["run", "flow.yaml"], &[("TMX_TEST_TOKEN", secret)]);
    assert_eq!(run.code, Some(0), "the secret run exits 0; {}", run.stderr);

    let list = run_tmx(&dir, &["runs", "list"], &[]);
    let ids = listed_ids(&list.stdout);
    let id = ids.first().expect("one run listed");

    let logs = run_tmx(&dir, &["runs", "logs", id], &[]);
    assert!(
        !logs.stdout.contains(secret),
        "the raw secret never appears in the replayed log"
    );
    assert!(
        logs.stdout.contains("[REDACTED]"),
        "the redaction placeholder replaces it, got {:?}",
        logs.stdout
    );

    let state = run_tmx(&dir, &["runs", "state", id], &[]);
    assert!(
        !state.stdout.contains(secret),
        "the raw secret never appears in the dumped state either"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn prune_removes_an_aged_record_and_keeps_a_fresh_one() {
    // O2: retention prunes an aged record while keeping a fresh one. An aged record is seeded directly
    // (its startedAt precedes the default 30-day window); a fresh run is then made, and `tmx runs prune`
    // removes only the aged one.
    let dir = temp_dir("prune");
    write(&dir, "flow.yaml", PASSING_YAML);

    // A fresh run (startedAt = now) that must survive the sweep.
    assert_eq!(run_tmx(&dir, &["run", "flow.yaml"], &[]).code, Some(0));
    let fresh = listed_ids(&run_tmx(&dir, &["runs", "list"], &[]).stdout)
        .first()
        .expect("the fresh run is listed")
        .clone();

    // Seed an aged record by hand: a valid UUIDv7 directory with an old startedAt.
    let aged = "018f0000-9b2a-7def-8123-456789abcdef";
    let aged_dir = dir.join(".tmx").join("runs").join(aged);
    std::fs::create_dir_all(&aged_dir).expect("create aged run dir");
    std::fs::write(
        aged_dir.join("record.json"),
        r#"{"id":"018f0000-9b2a-7def-8123-456789abcdef","flow":"old","status":"ok","startedAt":"2000-01-01T00:00:00.000Z"}"#,
    )
    .expect("seed aged record");

    // Both runs list before the prune.
    let before = listed_ids(&run_tmx(&dir, &["runs", "list"], &[]).stdout);
    assert_eq!(before.len(), 2, "the aged and fresh runs both list");

    // Prune with the default 30-day window: the aged record is removed, the fresh one kept.
    let prune = run_tmx(&dir, &["runs", "prune"], &[]);
    assert_eq!(prune.code, Some(0), "prune exits 0; {}", prune.stderr);
    let pruned: Value = serde_json::from_str(prune.stdout.trim()).expect("prune prints a count");
    assert_eq!(pruned["pruned"], 1, "exactly the aged record is pruned");

    let after = listed_ids(&run_tmx(&dir, &["runs", "list"], &[]).stdout);
    assert_eq!(after, vec![fresh], "only the fresh run survives the sweep");

    // Negative space: with retention disabled, prune removes nothing even on demand.
    std::fs::create_dir_all(&aged_dir).expect("re-create aged run dir");
    std::fs::write(
        aged_dir.join("record.json"),
        r#"{"id":"018f0000-9b2a-7def-8123-456789abcdef","flow":"old","status":"ok","startedAt":"2000-01-01T00:00:00.000Z"}"#,
    )
    .expect("re-seed aged record");
    let disabled = run_tmx(&dir, &["runs", "prune"], &[("TMX_RUNS_RETENTION", "off")]);
    let count: Value = serde_json::from_str(disabled.stdout.trim()).expect("prune prints a count");
    assert_eq!(count["pruned"], 0, "a disabled sweep prunes nothing");

    std::fs::remove_dir_all(&dir).ok();
}
