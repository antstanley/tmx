//! `tmx runs` — query the local run store (07 §Pipeline runs; 08 §Run store).
//!
//! Backs the `QueryRuns` driving port over the built-in [`LocalRunStore`]: `list` (chronological by
//! UUIDv7), `show` (the full masked record), `state` (the masked final-state snapshot), `logs`
//! (the replayed masked event log), `prune` (retention sweep on demand), and `rm` (remove one run).
//! Everything it dumps is the payload the store already persisted post-`Masker`, so a replay can never
//! re-expose a secret. `main` renders the returned JSON to stdout.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use tmx_adapters::clock::SystemClock;
use tmx_adapters::runstore::LocalRunStore;

use tmx_core::error::RunError;
use tmx_core::model::RunId;
use tmx_core::ports::driven::RunStore;
use tmx_core::ports::driving::{QueryRuns, RunQuery};

use crate::args::{RunsArgs, RunsCommand};
use crate::config;

/// The `QueryRuns` use case over a [`LocalRunStore`] — the read/prune side of the run store.
pub struct LocalQueryRuns {
    /// The store queried.
    store: Arc<LocalRunStore>,
    /// The retention window, in days, used by `prune`; `None` disables the sweep (returns 0).
    retention_days: Option<u64>,
}

impl LocalQueryRuns {
    /// Query `store`, pruning against `retention_days` (`None` disables the sweep).
    #[must_use]
    pub fn new(store: Arc<LocalRunStore>, retention_days: Option<u64>) -> Self {
        Self {
            store,
            retention_days,
        }
    }

    /// Read a run's persisted record, erroring with a typed `run_not_found` (resolution → exit 4) when
    /// no such run is stored — so `show`/`state`/`logs`/`rm` on a missing id give a clear failure
    /// rather than an empty success.
    fn require_record(&self, id: &RunId) -> Result<Value, RunError> {
        self.store.read_record_value(id)?.ok_or_else(|| {
            RunError::resolution(
                "run_not_found",
                format!("no run `{id}` is stored under ./.tmx/runs/"),
            )
        })
    }

    /// A one-line summary of a stored run for `list`: id, flow, status, timings.
    fn summary(&self, id: &RunId) -> Result<Value, RunError> {
        let record = self
            .store
            .read_record_value(id)?
            .unwrap_or_else(|| json!({}));
        Ok(json!({
            "id": id.as_str(),
            "flow": record.get("flow").cloned().unwrap_or(Value::Null),
            "status": record.get("status").cloned().unwrap_or(Value::Null),
            "startedAt": record.get("startedAt").cloned().unwrap_or(Value::Null),
            "ms": record.get("ms").cloned().unwrap_or(Value::Null),
        }))
    }
}

#[async_trait]
impl QueryRuns for LocalQueryRuns {
    async fn query(&self, query: RunQuery) -> Result<Value, RunError> {
        match query {
            RunQuery::List => {
                let mut runs = Vec::new();
                for id in self.store.list().await? {
                    runs.push(self.summary(&id)?);
                }
                Ok(json!({ "runs": runs }))
            }
            RunQuery::Show(id) => self.require_record(&id),
            RunQuery::State(id) => {
                let record = self.require_record(&id)?;
                Ok(record
                    .get("finalState")
                    .cloned()
                    .unwrap_or_else(|| json!({})))
            }
            RunQuery::Logs(id) => {
                // Confirm the run exists before replaying so a missing id is a clear error, not an
                // empty log.
                self.require_record(&id)?;
                let events = self.store.read_log_values(&id)?;
                Ok(json!({ "events": events }))
            }
            RunQuery::Prune => {
                let pruned = match self.retention_days {
                    // Retention disabled: prune nothing, even on demand.
                    None => 0,
                    Some(days) => {
                        let cutoff = SystemClock::new().cutoff_days_ago(days);
                        self.store.prune(&cutoff).await?
                    }
                };
                Ok(json!({ "pruned": pruned }))
            }
            RunQuery::Remove(id) => {
                // Removing a missing run is a clear error here (unlike the idempotent port method), so
                // `tmx runs rm <unknown>` tells the operator nothing matched.
                self.require_record(&id)?;
                self.store.remove(&id).await?;
                Ok(json!({ "removed": id.as_str() }))
            }
        }
    }
}

/// Run the `tmx runs` command to its JSON result (or a typed [`RunError`]).
///
/// # Errors
///
/// Returns a `validation` [`RunError`] for a malformed run id, a `run_not_found` (`resolution`) for a
/// missing run, or any store read/write failure the query surfaces.
pub async fn execute(args: RunsArgs) -> Result<Value, RunError> {
    let cwd = std::env::current_dir().map_err(|e| {
        RunError::resolution(
            "cwd_unavailable",
            format!("could not read the current working directory: {e}"),
        )
    })?;
    let base: PathBuf = cwd.join(".tmx").join("runs");
    let store = Arc::new(LocalRunStore::new(base));
    let queries = LocalQueryRuns::new(store, config::resolve_retention_days());

    let query = match args.command {
        RunsCommand::List => RunQuery::List,
        RunsCommand::Show { id } => RunQuery::Show(RunId::new(id)?),
        RunsCommand::State { id } => RunQuery::State(RunId::new(id)?),
        RunsCommand::Logs { id } => RunQuery::Logs(RunId::new(id)?),
        RunsCommand::Prune => RunQuery::Prune,
        RunsCommand::Rm { id } => RunQuery::Remove(RunId::new(id)?),
    };
    queries.query(query).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    use tmx_core::model::{Milliseconds, PipelineState, RunRecord, RunStatus, Timestamp};

    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a synchronous-body future must complete on first poll"),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tmx-runs-cmd-{tag}-{}-{:p}",
            std::process::id(),
            &tag
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

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
            ms: Some(Milliseconds(9)),
            final_state: Some(PipelineState::new(serde_json::json!({"answer": 42})).unwrap()),
            results: Vec::new(),
        }
    }

    #[test]
    fn list_is_chronological_and_state_logs_dump_the_masked_snapshot() {
        let dir = temp_dir("list");
        let store = Arc::new(LocalRunStore::new(&dir));
        let early = id_for("018f0000");
        let late = id_for("018fffff");
        block_on_ready(store.save(&record(&late, "second", "2026-07-05T10:00:00.000Z"))).unwrap();
        block_on_ready(store.save(&record(&early, "first", "2026-07-05T09:00:00.000Z"))).unwrap();
        block_on_ready(store.append_event(
            &early,
            &tmx_core::model::Event::RunStart {
                id: early.clone(),
                flow: "first".to_string(),
            },
        ))
        .unwrap();

        let queries = LocalQueryRuns::new(Arc::clone(&store), Some(30));

        // list is chronological by id.
        let listed = block_on_ready(queries.query(RunQuery::List)).expect("list");
        let runs = listed["runs"].as_array().expect("an array of runs");
        assert_eq!(runs.len(), 2, "both runs listed");
        assert_eq!(runs[0]["id"], early.as_str(), "the earlier run sorts first");
        assert_eq!(runs[1]["id"], late.as_str(), "the later run sorts second");

        // state dumps the final state.
        let state = block_on_ready(queries.query(RunQuery::State(early.clone()))).expect("state");
        assert_eq!(state["answer"], 42, "state dumps the masked final state");

        // logs replays the event log.
        let logs = block_on_ready(queries.query(RunQuery::Logs(early.clone()))).expect("logs");
        let events = logs["events"].as_array().expect("an events array");
        assert_eq!(events.len(), 1, "the one event replays");
        assert_eq!(events[0]["event"], "run.start", "the event tag replays");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_run_is_a_typed_not_found_and_rm_removes() {
        // Negative space: show/state/logs/rm on an unknown id is a resolution error, not empty success.
        let dir = temp_dir("missing");
        let store = Arc::new(LocalRunStore::new(&dir));
        let queries = LocalQueryRuns::new(Arc::clone(&store), Some(30));
        let ghost = id_for("018fabcd");

        let err = block_on_ready(queries.query(RunQuery::Show(ghost.clone())))
            .expect_err("show of a missing run errors");
        assert_eq!(err.code, "run_not_found", "the missing-run code");
        assert_eq!(
            err.category,
            tmx_core::ErrorCategory::Resolution,
            "a missing run is a resolution error (exit 4)"
        );

        // rm removes a present run and then reports it gone.
        let present = id_for("018f1111");
        block_on_ready(store.save(&record(&present, "keep", "2026-07-05T10:00:00.000Z"))).unwrap();
        let removed =
            block_on_ready(queries.query(RunQuery::Remove(present.clone()))).expect("rm present");
        assert_eq!(
            removed["removed"],
            present.as_str(),
            "rm reports the id removed"
        );
        assert!(
            store
                .read_record_value(&present)
                .expect("read after rm")
                .is_none(),
            "the run is gone after rm"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_is_a_noop_when_retention_is_disabled() {
        // Negative space: with retention disabled, on-demand prune removes nothing.
        let dir = temp_dir("prune-off");
        let store = Arc::new(LocalRunStore::new(&dir));
        let aged = id_for("018f0000");
        block_on_ready(store.save(&record(&aged, "old", "2000-01-01T00:00:00.000Z"))).unwrap();

        let disabled = LocalQueryRuns::new(Arc::clone(&store), None);
        let result = block_on_ready(disabled.query(RunQuery::Prune)).expect("prune disabled");
        assert_eq!(result["pruned"], 0, "a disabled sweep prunes nothing");
        assert_eq!(
            block_on_ready(store.list()).unwrap().len(),
            1,
            "the aged run survives a disabled sweep"
        );

        // With a window, the aged run prunes.
        let enabled = LocalQueryRuns::new(Arc::clone(&store), Some(1));
        let result = block_on_ready(enabled.query(RunQuery::Prune)).expect("prune enabled");
        assert_eq!(result["pruned"], 1, "the aged run prunes with a window");

        std::fs::remove_dir_all(&dir).ok();
    }
}
