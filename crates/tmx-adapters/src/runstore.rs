//! [`LocalRunStore`] — the local-filesystem [`RunStore`] adapter (`./.tmx/runs/<uuidv7>/`).
//!
//! Persists each run as a **record, not a journal** (08 §Run store): a final-state snapshot
//! (`record.json`, the serialised [`RunRecord`]) plus the newline-delimited event log
//! (`log.ndjson`), under a directory keyed by the run's time-ordered [`RunId`] (a UUIDv7) so a
//! directory listing is chronological without a separate sort key.
//!
//! Two bounds shape the log, both from `tmx-schema::limits`:
//!
//! - **The event log is capped, not sampled.** Appends are bounded by
//!   [`EVENT_LOG_MAX_BYTES`](tmx_schema::limits::EVENT_LOG_MAX_BYTES): on overflow the store writes a
//!   final `log.truncated` event and stops persisting further events for that run, while the caller's
//!   stdout streaming and the final-state snapshot continue unaffected. The record is truncated at the
//!   byte boundary, never down-sampled.
//! - **Retention.** [`prune`](LocalRunStore::prune) removes every run whose `startedAt` precedes a
//!   cutoff [`Timestamp`] — the sweep the CLI runs opportunistically at each `tmx run` and on demand
//!   via `tmx runs prune`, defaulting to
//!   [`RUN_RETENTION_DEFAULT_DAYS`](tmx_schema::limits::RUN_RETENTION_DEFAULT_DAYS) days.
//!
//! Everything the store persists is the caller's already-[`Masker`](tmx_core::mask::Masker)-scrubbed
//! payload (the tee wraps the reporter, which only ever sees `Masked` events, and `save` records the
//! `RunRecord` whose final state the engine masked before it left the core), so a later replay through
//! `tmx runs state`/`logs` cannot re-expose a secret. The store reaches only for [`std::fs`]; it
//! carries no async-runtime or heavy-I/O edge.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use tmx_core::error::{ErrorCategory, RunError};
use tmx_core::mask::Masked;
use tmx_core::model::{Event, Milliseconds, PipelineState, RunId, RunRecord, RunStatus, Timestamp};
use tmx_core::ports::driven::{EventSink, RunStore};
use tmx_schema::limits::EVENT_LOG_MAX_BYTES;

/// The final-state snapshot file within a run's directory — the serialised [`RunRecord`].
const RECORD_FILE: &str = "record.json";
/// The newline-delimited event log file within a run's directory.
const LOG_FILE: &str = "log.ndjson";

/// Persists, queries, and prunes runs against the local filesystem — the built-in [`RunStore`].
///
/// Holds the base directory every run is stored under (`./.tmx/runs`), the per-append log cap (default
/// [`EVENT_LOG_MAX_BYTES`], overridable for tests), and the per-run persisted-byte bookkeeping the cap
/// is enforced against. Constructed once per process; shared behind an `Arc` between the storing event
/// tee and the run command.
#[derive(Debug)]
pub struct LocalRunStore {
    /// The directory every run's `<id>/` subdirectory lives under.
    base: PathBuf,
    /// The per-run event-log ceiling, in bytes; reaching it writes `log.truncated` and stops.
    log_cap_bytes: u64,
    /// Per-run log bookkeeping (bytes persisted so far, and whether the cap has stopped the log),
    /// keyed by the run id's string form. Interior mutability so the `&self` port methods can record.
    logs: Mutex<HashMap<String, LogState>>,
}

/// One run's event-log bookkeeping: how many bytes have been persisted, and whether the cap has
/// already stopped further persistence (after a `log.truncated` marker).
#[derive(Debug, Default, Clone, Copy)]
struct LogState {
    /// Bytes of event JSON persisted to this run's log so far.
    bytes: u64,
    /// Whether the cap has been reached and persistence stopped for this run.
    stopped: bool,
}

impl LocalRunStore {
    /// A store rooted at `base` (typically `./.tmx/runs`), with the default event-log cap
    /// [`EVENT_LOG_MAX_BYTES`].
    #[must_use]
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            log_cap_bytes: EVENT_LOG_MAX_BYTES,
            logs: Mutex::new(HashMap::new()),
        }
    }

    /// A store with an explicit event-log cap, in bytes — for tests exercising the `log.truncated`
    /// path without writing a 256 MiB log.
    #[must_use]
    pub fn with_log_cap_bytes(base: impl Into<PathBuf>, log_cap_bytes: u64) -> Self {
        Self {
            base: base.into(),
            log_cap_bytes,
            logs: Mutex::new(HashMap::new()),
        }
    }

    /// The directory a run's artifacts live under (`<base>/<id>/`).
    fn run_dir(&self, id: &RunId) -> PathBuf {
        self.base.join(id.as_str())
    }

    /// Ensure a run's directory exists, mapping a failure to a typed [`RunError`].
    fn ensure_run_dir(&self, id: &RunId) -> Result<PathBuf, RunError> {
        let dir = self.run_dir(id);
        std::fs::create_dir_all(&dir).map_err(|e| write_error(&dir, e))?;
        Ok(dir)
    }

    /// Read a run's persisted `record.json` as a raw JSON [`Value`], or `None` when the run (or its
    /// record) is not present. The value is exactly what was persisted — already masked — so a caller
    /// replaying it cannot re-expose a secret.
    ///
    /// # Errors
    ///
    /// Returns a `run_store_read_failed` [`RunError`] when the record exists but cannot be read or
    /// parsed as JSON.
    pub fn read_record_value(&self, id: &RunId) -> Result<Option<Value>, RunError> {
        let path = self.run_dir(id).join(RECORD_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let value = serde_json::from_slice(&bytes).map_err(|e| {
                    RunError::run_failure(
                        "run_store_read_failed",
                        format!("could not parse `{}` as JSON: {e}", path.display()),
                    )
                })?;
                Ok(Some(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(read_error(&path, e)),
        }
    }

    /// Read a run's persisted event log as the parsed JSON [`Value`] of each `log.ndjson` line, in
    /// order. An empty vector when the run has no log. Every line is already masked.
    ///
    /// # Errors
    ///
    /// Returns a `run_store_read_failed` [`RunError`] when the log exists but cannot be read, or a line
    /// is not valid JSON.
    pub fn read_log_values(&self, id: &RunId) -> Result<Vec<Value>, RunError> {
        let path = self.run_dir(id).join(LOG_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(read_error(&path, e)),
        };
        let mut events = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line).map_err(|e| {
                RunError::run_failure(
                    "run_store_read_failed",
                    format!(
                        "could not parse a log line in `{}` as JSON: {e}",
                        path.display()
                    ),
                )
            })?;
            events.push(value);
        }
        Ok(events)
    }

    /// The `startedAt` timestamp a run's record carries, when present — the key retention compares
    /// against its cutoff. `None` when the run has no readable record or no `startedAt`.
    fn started_at(&self, id: &RunId) -> Option<String> {
        let value = self.read_record_value(id).ok()??;
        value
            .get("startedAt")
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

#[async_trait]
impl RunStore for LocalRunStore {
    async fn save(&self, record: &RunRecord) -> Result<(), RunError> {
        // The work is synchronous `std::fs`; the `async` boundary is the port contract.
        let dir = self.ensure_run_dir(&record.id)?;
        let path = dir.join(RECORD_FILE);
        let bytes = serde_json::to_vec_pretty(record).map_err(|e| {
            RunError::run_failure(
                "run_store_write_failed",
                format!("could not serialise the run record: {e}"),
            )
        })?;
        std::fs::write(&path, &bytes).map_err(|e| write_error(&path, e))
    }

    async fn append_event(&self, id: &RunId, event: &Event) -> Result<(), RunError> {
        let line = serde_json::to_string(event).map_err(|e| {
            RunError::run_failure(
                "run_store_write_failed",
                format!("could not serialise an event to the log: {e}"),
            )
        })?;

        // Decide, under the lock, whether this event fits under the cap or trips the truncation
        // boundary. The lock is held only for the bookkeeping decision, never across the file write's
        // error handling in a way that could poison it on a genuine I/O failure.
        let mut logs = self
            .logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = logs.entry(id.as_str().to_string()).or_default();
        if state.stopped {
            // The cap already stopped this run's log; drop silently (streaming continues elsewhere).
            return Ok(());
        }

        let event_bytes = line.len() as u64 + 1; // +1 for the trailing newline.
        if state.bytes.saturating_add(event_bytes) > self.log_cap_bytes {
            // Reaching the cap is not an error: write one final `log.truncated` marker, stop, and
            // leave the run to continue. The record is capped at the byte boundary, never sampled.
            state.stopped = true;
            drop(logs);
            let marker = serde_json::to_string(&Event::LogTruncated).map_err(|e| {
                RunError::run_failure(
                    "run_store_write_failed",
                    format!("could not serialise the log.truncated marker: {e}"),
                )
            })?;
            return self.append_line(id, &marker);
        }
        state.bytes = state.bytes.saturating_add(event_bytes);
        drop(logs);
        self.append_line(id, &line)
    }

    async fn list(&self) -> Result<Vec<RunId>, RunError> {
        let read = match std::fs::read_dir(&self.base) {
            Ok(read) => read,
            // No store directory yet is an empty listing, not an error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(read_error(&self.base, e)),
        };
        let mut ids: Vec<String> = Vec::new();
        for entry in read {
            let Ok(entry) = entry else { continue };
            if !entry.path().is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            // Only well-formed UUIDv7 directory names are runs; anything else is ignored, never an
            // error, so a stray file cannot corrupt a listing.
            if RunId::new(name.clone()).is_ok() {
                ids.push(name);
            }
        }
        // UUIDv7 is time-ordered, so a lexical sort is chronological — no separate sort key needed.
        ids.sort();
        Ok(ids.into_iter().filter_map(|s| RunId::new(s).ok()).collect())
    }

    async fn get(&self, id: &RunId) -> Result<Option<RunRecord>, RunError> {
        let Some(value) = self.read_record_value(id)? else {
            return Ok(None);
        };
        let stored: StoredRecord = serde_json::from_value(value).map_err(|e| {
            RunError::run_failure(
                "run_store_read_failed",
                format!("a persisted run record is malformed: {e}"),
            )
        })?;
        Ok(Some(stored.into_record()?))
    }

    async fn prune(&self, cutoff: &Timestamp) -> Result<u32, RunError> {
        let mut removed: u32 = 0;
        for id in self.list().await? {
            // A run started strictly before the cutoff is aged out. `startedAt` and the cutoff are both
            // fixed-width RFC 3339 UTC instants, so a lexical comparison is chronological. A run with no
            // readable `startedAt` is kept (never pruned on missing data).
            let Some(started) = self.started_at(&id) else {
                continue;
            };
            if started.as_str() < cutoff.as_str() {
                let dir = self.run_dir(&id);
                std::fs::remove_dir_all(&dir).map_err(|e| write_error(&dir, e))?;
                self.logs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(id.as_str());
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }

    async fn remove(&self, id: &RunId) -> Result<(), RunError> {
        let dir = self.run_dir(id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            // Removing an absent run is idempotent, not an error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(write_error(&dir, e)),
        }
        self.logs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id.as_str());
        Ok(())
    }
}

impl LocalRunStore {
    /// Append one already-serialised event line (plus a newline) to a run's `log.ndjson`, creating the
    /// run directory if needed. The byte-cap decision has already been made by the caller.
    fn append_line(&self, id: &RunId, line: &str) -> Result<(), RunError> {
        let dir = self.ensure_run_dir(id)?;
        let path = dir.join(LOG_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| write_error(&path, e))?;
        writeln!(file, "{line}").map_err(|e| write_error(&path, e))
    }
}

/// An [`EventSink`] that tees the masked event stream to an inner reporter **and** persists each event
/// to a [`LocalRunStore`] — the seam by which the store *records from* the reporter stream rather than
/// replacing it (08 §Run store: "It is a record, not a journal").
///
/// The inner reporter runs first and its result is propagated (the stdout data contract is
/// load-bearing); the store append runs second and its failure is **swallowed to stderr** — an
/// observability-overflow or a disk hiccup must not abort the run (08 §Cancellation: "observability
/// overflow should not abort work"). The run id is learned from the first `run.start` event (always the
/// first event a run emits) and held for the whole run, so every subsequent event — including a nested
/// sub-flow's — is filed under the top-level run.
pub struct StoringSink {
    /// The reporter the stream is teed to first (unchanged behaviour).
    inner: Box<dyn EventSink>,
    /// The store each masked event is persisted to.
    store: std::sync::Arc<LocalRunStore>,
    /// The top-level run id, learned once from the first `run.start`.
    run_id: Mutex<Option<RunId>>,
}

impl StoringSink {
    /// Wrap `inner`, persisting each masked event to `store`.
    #[must_use]
    pub fn new(inner: Box<dyn EventSink>, store: std::sync::Arc<LocalRunStore>) -> Self {
        Self {
            inner,
            store,
            run_id: Mutex::new(None),
        }
    }

    /// The run id to file an event under: the one learned from `run.start`, latching it on first sight.
    fn run_id_for(&self, event: &Event) -> Option<RunId> {
        let mut guard = self
            .run_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_none()
            && let Event::RunStart { id, .. } = event
        {
            *guard = Some(id.clone());
        }
        guard.clone()
    }
}

#[async_trait]
impl EventSink for StoringSink {
    async fn emit(&self, event: &Masked<Event>) -> Result<(), RunError> {
        // Stream first: the reporter's stdout/stderr output is the load-bearing contract, unchanged.
        self.inner.emit(event).await?;
        // Then persist the already-masked event. Persistence is best-effort — its failure must not
        // abort a run whose observable output already succeeded.
        if let Some(id) = self.run_id_for(event.get())
            && let Err(e) = self.store.append_event(&id, event.get()).await
        {
            eprintln!("tmx: warning: could not persist a run event: {}", e.message);
        }
        Ok(())
    }
}

/// A permissive shadow of a persisted [`RunRecord`], used to read a record back from disk (the model's
/// emit-only types carry no `Deserialize`; the `RunStore` owns the read-back, 04 §model). Field names
/// mirror the camelCase the record is serialised with.
#[derive(Debug, Deserialize)]
struct StoredRecord {
    id: String,
    #[serde(default)]
    flow: Option<String>,
    status: RunStatus,
    #[serde(rename = "startedAt")]
    started_at: String,
    #[serde(rename = "finishedAt", default)]
    finished_at: Option<String>,
    #[serde(default)]
    ms: Option<u64>,
    #[serde(rename = "finalState", default)]
    final_state: Option<Value>,
    #[serde(default)]
    results: Vec<StoredResult>,
}

/// A permissive shadow of a persisted [`TaskResult`](tmx_core::model::TaskResult).
#[derive(Debug, Deserialize)]
struct StoredResult {
    name: String,
    status: tmx_core::model::TaskStatus,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    error: Option<StoredError>,
    #[serde(rename = "startedAt")]
    started_at: String,
    ms: u64,
}

/// A permissive shadow of a persisted [`RunError`].
#[derive(Debug, Deserialize)]
struct StoredError {
    category: ErrorCategory,
    code: String,
    message: String,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

impl StoredRecord {
    /// Rebuild the typed [`RunRecord`] from the persisted shadow, validating the run id.
    fn into_record(self) -> Result<RunRecord, RunError> {
        let final_state = match self.final_state {
            Some(value) => Some(PipelineState::new(value)?),
            None => None,
        };
        let results = self
            .results
            .into_iter()
            .map(StoredResult::into_result)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RunRecord {
            id: RunId::new(self.id)?,
            flow: self.flow,
            status: self.status,
            started_at: Timestamp::new(self.started_at),
            finished_at: self.finished_at.map(Timestamp::new),
            ms: self.ms.map(Milliseconds),
            final_state,
            results,
        })
    }
}

impl StoredResult {
    /// Rebuild the typed [`TaskResult`](tmx_core::model::TaskResult) from the persisted shadow.
    fn into_result(self) -> Result<tmx_core::model::TaskResult, RunError> {
        Ok(tmx_core::model::TaskResult {
            name: self.name,
            status: self.status,
            output: self.output,
            error: self.error.map(StoredError::into_error),
            started_at: Timestamp::new(self.started_at),
            ms: Milliseconds(self.ms),
        })
    }
}

impl StoredError {
    /// Rebuild a typed [`RunError`] from the persisted shadow, interning its stable code to the
    /// `&'static str` the type requires (see [`intern_code`]).
    fn into_error(self) -> RunError {
        let mut error = RunError::new(self.category, intern_code(&self.code), self.message);
        if let Some(task) = self.task {
            error = error.with_task(task);
        }
        if let Some(path) = self.path {
            error = error.with_path(path);
        }
        error
    }
}

/// Intern a persisted error code to the `&'static str` [`RunError::code`] requires.
///
/// Error codes are a small, stable vocabulary; the interner deduplicates so a distinct code is leaked
/// at most once for the process's lifetime — bounded, and reclaimed at exit. The read-back path (a
/// short-lived `tmx runs show`) never accumulates unbounded leakage.
fn intern_code(code: &str) -> &'static str {
    static INTERN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let set = INTERN.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = set
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = guard.get(code) {
        return existing;
    }
    let leaked: &'static str = Box::leak(code.to_owned().into_boxed_str());
    guard.insert(leaked);
    leaked
}

/// A write / create failure against `path`, as a typed [`RunError`].
fn write_error(path: &Path, source: std::io::Error) -> RunError {
    RunError::run_failure(
        "run_store_write_failed",
        format!("could not write `{}`: {source}", path.display()),
    )
    .with_path(path.display().to_string())
}

/// A read failure against `path`, as a typed [`RunError`].
fn read_error(path: &Path, source: std::io::Error) -> RunError {
    RunError::run_failure(
        "run_store_read_failed",
        format!("could not read `{}`: {source}", path.display()),
    )
    .with_path(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use tmx_core::mask::Masker;
    use tmx_core::model::{TaskResult, TaskStatus};

    /// Drive a ready future to completion with a no-op waker — the adapter's futures never yield (the
    /// work is synchronous `std::fs`), so they are ready on the first poll.
    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a synchronous-body future must complete on first poll"),
        }
    }

    /// A unique temp directory for one test.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tmx-runstore-{tag}-{}-{:p}",
            std::process::id(),
            &tag
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// A well-formed UUIDv7 with a controllable leading byte so ids sort in a known order.
    fn id_for(lead: &str) -> RunId {
        RunId::new(format!("{lead}-9b2a-7def-8123-456789abcdef")).expect("a valid UUIDv7")
    }

    fn record(id: &RunId, flow: &str, started_at: &str) -> RunRecord {
        RunRecord {
            id: id.clone(),
            flow: Some(flow.to_string()),
            status: RunStatus::Ok,
            started_at: Timestamp::new(started_at),
            finished_at: Some(Timestamp::new(started_at)),
            ms: Some(Milliseconds(7)),
            final_state: Some(PipelineState::new(serde_json::json!({"k": "v"})).unwrap()),
            results: Vec::new(),
        }
    }

    #[test]
    fn persists_a_run_and_lists_chronologically_then_reads_state_and_logs() {
        let dir = temp_dir("persist");
        let store = LocalRunStore::new(&dir);

        // Two runs, saved out of chronological order; UUIDv7 lead bytes give the intended order.
        let early = id_for("018f8c7e");
        let late = id_for("018f9000");
        block_on_ready(store.save(&record(&late, "second", "2026-07-05T10:00:00.000Z")))
            .expect("save late");
        block_on_ready(store.save(&record(&early, "first", "2026-07-05T09:00:00.000Z")))
            .expect("save early");

        // An event is appended and read back masked.
        let masker = Masker::new();
        let masked = masker.redact_event(&Event::RunStart {
            id: early.clone(),
            flow: "first".to_string(),
        });
        block_on_ready(store.append_event(&early, masked.get())).expect("append event");

        // Listing is chronological by UUIDv7 without a sort key.
        let listed = block_on_ready(store.list()).expect("list");
        assert_eq!(listed.len(), 2, "both runs are listed");
        assert_eq!(listed[0], early, "the earlier run sorts first");
        assert_eq!(listed[1], late, "the later run sorts second");

        // `state` reads the masked final-state snapshot back.
        let value = store
            .read_record_value(&early)
            .expect("read record")
            .expect("record present");
        assert_eq!(value["finalState"]["k"], "v", "the final state round-trips");

        // `logs` replays the persisted event log.
        let events = store.read_log_values(&early).expect("read log");
        assert_eq!(events.len(), 1, "the one appended event is replayed");
        assert_eq!(events[0]["event"], "run.start", "the event tag round-trips");

        // The typed `get` reconstructs the record (id/flow/status/state).
        let got = block_on_ready(store.get(&early))
            .expect("get")
            .expect("record present");
        assert_eq!(got.id, early, "the id round-trips through get");
        assert_eq!(got.flow.as_deref(), Some("first"), "the flow round-trips");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_run_reads_as_none_and_removing_it_is_idempotent() {
        // Negative space: querying and removing an absent run is not an error.
        let dir = temp_dir("missing");
        let store = LocalRunStore::new(&dir);
        let ghost = id_for("018fabcd");

        assert!(
            block_on_ready(store.get(&ghost))
                .expect("get absent")
                .is_none(),
            "an absent run reads back as None, not an error"
        );
        assert!(
            store
                .read_record_value(&ghost)
                .expect("read absent")
                .is_none(),
            "an absent record value is None"
        );
        assert!(
            store
                .read_log_values(&ghost)
                .expect("read absent log")
                .is_empty(),
            "an absent log is empty"
        );
        block_on_ready(store.remove(&ghost)).expect("removing an absent run is idempotent");
        // An empty/absent base directory lists nothing.
        assert!(
            block_on_ready(store.list()).expect("list").is_empty(),
            "no runs are listed for an empty store"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_event_log_is_capped_with_a_truncated_marker_and_then_stops() {
        // Drive the log to a tiny cap: once reached, a final `log.truncated` is written and no further
        // event persists — the record is capped at the byte boundary, never sampled.
        let dir = temp_dir("cap");
        let store = LocalRunStore::with_log_cap_bytes(&dir, 40);
        let id = id_for("018f8c7e");
        let masker = Masker::new();

        let mut appended = 0;
        for i in 0..50 {
            let masked = masker.redact_event(&Event::TaskStart {
                name: format!("t{i}"),
            });
            block_on_ready(store.append_event(&id, masked.get())).expect("append");
            appended += 1;
            if appended > 20 {
                break;
            }
        }

        let events = store.read_log_values(&id).expect("read log");
        // The log stopped short of the 21 appends, and its last line is the truncation marker.
        assert!(
            events.len() < 21,
            "the log is capped, not the full stream: {} lines",
            events.len()
        );
        let last = events.last().expect("at least one line");
        assert_eq!(
            last["event"], "log.truncated",
            "the final persisted event is the truncation marker"
        );
        // Exactly one truncation marker — persistence stopped, it did not keep re-emitting.
        let markers = events
            .iter()
            .filter(|e| e["event"] == "log.truncated")
            .count();
        assert_eq!(markers, 1, "the truncation marker is written exactly once");

        // The final-state snapshot still saves after the log capped — the run is unaffected.
        block_on_ready(store.save(&record(&id, "capped", "2026-07-05T10:00:00.000Z")))
            .expect("save after cap");
        assert!(
            store.read_record_value(&id).expect("read").is_some(),
            "the final-state snapshot persists even after the log capped"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn retention_prunes_aged_runs_and_keeps_fresh_ones() {
        let dir = temp_dir("retention");
        let store = LocalRunStore::new(&dir);
        let aged = id_for("018f0000");
        let fresh = id_for("018fffff");
        block_on_ready(store.save(&record(&aged, "old", "2026-01-01T00:00:00.000Z")))
            .expect("save aged");
        block_on_ready(store.save(&record(&fresh, "new", "2026-07-05T00:00:00.000Z")))
            .expect("save fresh");

        // Cutoff between the two: the aged run is pruned, the fresh one kept.
        let cutoff = Timestamp::new("2026-06-01T00:00:00.000Z");
        let removed = block_on_ready(store.prune(&cutoff)).expect("prune");
        assert_eq!(removed, 1, "exactly the aged run is pruned");
        let listed = block_on_ready(store.list()).expect("list");
        assert_eq!(listed, vec![fresh], "only the fresh run survives");

        // Negative space: a cutoff before every run prunes nothing.
        let past = Timestamp::new("2020-01-01T00:00:00.000Z");
        assert_eq!(
            block_on_ready(store.prune(&past)).expect("prune past"),
            0,
            "a cutoff before every run prunes nothing"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn get_reconstructs_a_failed_task_result_with_its_error() {
        // A record with a failed task round-trips through get, error code and message intact.
        let dir = temp_dir("get-error");
        let store = LocalRunStore::new(&dir);
        let id = id_for("018f8c7e");
        let mut rec = record(&id, "flow", "2026-07-05T10:00:00.000Z");
        rec.status = RunStatus::Failed;
        rec.results = vec![TaskResult {
            name: "boom".to_string(),
            status: TaskStatus::Error,
            output: None,
            error: Some(RunError::run_failure(
                "assert_failed",
                "the assertion did not hold",
            )),
            started_at: Timestamp::new("2026-07-05T10:00:00.000Z"),
            ms: Milliseconds(3),
        }];
        block_on_ready(store.save(&rec)).expect("save");

        let got = block_on_ready(store.get(&id))
            .expect("get")
            .expect("present");
        assert_eq!(got.status, RunStatus::Failed, "the status round-trips");
        assert_eq!(got.results.len(), 1, "the result round-trips");
        let err = got.results[0]
            .error
            .as_ref()
            .expect("the error round-trips");
        assert_eq!(err.code, "assert_failed", "the error code round-trips");
        assert_eq!(
            err.message, "the assertion did not hold",
            "the error message round-trips"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_storing_sink_tees_the_stream_and_persists_masked_events() {
        // The tee records from the reporter stream: after emitting run.start then run.finish through a
        // StoringSink, both are persisted under the run id learned from run.start.
        let dir = temp_dir("tee");
        let store = Arc::new(LocalRunStore::new(&dir));
        let sink = StoringSink::new(Box::new(CountingSink::default()), Arc::clone(&store));
        let id = id_for("018f8c7e");
        let masker = Masker::new();

        block_on_ready(sink.emit(&masker.redact_event(&Event::RunStart {
            id: id.clone(),
            flow: "deploy".to_string(),
        })))
        .expect("emit run.start");
        block_on_ready(sink.emit(&masker.redact_event(&Event::RunFinish {
            id: id.clone(),
            status: RunStatus::Ok,
            ms: Milliseconds(5),
        })))
        .expect("emit run.finish");

        let events = store.read_log_values(&id).expect("read log");
        assert_eq!(events.len(), 2, "both events are persisted from the stream");
        assert_eq!(events[0]["event"], "run.start", "run.start persisted first");
        assert_eq!(
            events[1]["event"], "run.finish",
            "run.finish persisted second"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A minimal inner [`EventSink`] that counts emissions — the tee's downstream in tests.
    #[derive(Debug, Default)]
    struct CountingSink {
        count: Mutex<u32>,
    }

    #[async_trait]
    impl EventSink for CountingSink {
        async fn emit(&self, _event: &Masked<Event>) -> Result<(), RunError> {
            *self
                .count
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            Ok(())
        }
    }
}
