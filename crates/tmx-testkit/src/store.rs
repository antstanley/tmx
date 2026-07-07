//! [`MemObjectStore`] and [`MemRunStore`] — the in-memory `store` and `RunStore` fakes.
//!
//! Both back their port with a [`BTreeMap`] behind a [`Mutex`], so listings come back in a
//! deterministic key order (for the run store, that is the UUIDv7 lexical order the real store
//! relies on to sort runs chronologically). A missing key on a `get`/`head` object read is a typed
//! [`RunError`], not a panic.

use std::collections::BTreeMap;
use std::sync::Mutex;

use tmx_core::Milliseconds;
use tmx_core::ports::driven::{ObjectStore, RunStore, StoreOp, StoreResult};
use tmx_core::{Event, RunError, RunId, RunRecord, Timestamp};

/// An in-memory [`ObjectStore`]: keys map to object bytes in sorted order.
#[derive(Debug, Default)]
pub struct MemObjectStore {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemObjectStore {
    /// An empty object store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed an object at `key` with `body` before the run (builder form).
    #[must_use]
    pub fn with_object(self, key: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        if let Ok(mut objects) = self.objects.lock() {
            objects.insert(key.into(), body.into());
        }
        self
    }
}

/// The typed error an object read raises when `key` is absent.
fn object_not_found(key: &str) -> RunError {
    RunError::run_failure(
        "object_not_found",
        format!("no such object in memory: {key}"),
    )
}

/// A poisoned in-memory lock is an internal fault, surfaced as a typed error rather than a panic.
fn lock_poisoned(what: &'static str) -> RunError {
    RunError::run_failure(
        "store_lock_poisoned",
        format!("the in-memory {what} lock was poisoned"),
    )
}

#[async_trait::async_trait]
impl ObjectStore for MemObjectStore {
    // The in-memory store is instantaneous, so the per-op `timeout` is accepted (the port contract)
    // and never breached; the real S3 adapter is where a slow endpoint surfaces `task_timeout`.
    async fn op(
        &self,
        op: StoreOp,
        _timeout: Option<Milliseconds>,
    ) -> Result<StoreResult, RunError> {
        let mut objects = self
            .objects
            .lock()
            .map_err(|_| lock_poisoned("object store"))?;
        match op {
            StoreOp::Get { key } => {
                let body = objects
                    .get(&key)
                    .ok_or_else(|| object_not_found(&key))?
                    .clone();
                Ok(StoreResult::Get { body })
            }
            StoreOp::Put { key, body } => {
                objects.insert(key, body);
                Ok(StoreResult::Done)
            }
            StoreOp::Delete { key } => {
                objects.remove(&key);
                Ok(StoreResult::Done)
            }
            StoreOp::List { prefix } => {
                // BTreeMap iterates in sorted key order, so the listing is deterministic.
                let keys = objects
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .cloned()
                    .collect();
                Ok(StoreResult::List { keys })
            }
            StoreOp::Head { key } => {
                let entry = objects.get(&key);
                Ok(StoreResult::Head {
                    exists: entry.is_some(),
                    size_bytes: entry.map(|b| b.len() as u64),
                })
            }
        }
    }
}

/// An in-memory [`RunStore`]: records and per-run event logs keyed by run id in sorted (UUIDv7,
/// hence chronological) order.
#[derive(Debug, Default)]
pub struct MemRunStore {
    records: Mutex<BTreeMap<String, RunRecord>>,
    events: Mutex<BTreeMap<String, Vec<Event>>>,
}

impl MemRunStore {
    /// An empty run store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The events persisted for run `id`, in append order, for post-run assertions.
    #[must_use]
    pub fn events_for(&self, id: &RunId) -> Vec<Event> {
        self.events
            .lock()
            .ok()
            .and_then(|e| e.get(id.as_str()).cloned())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl RunStore for MemRunStore {
    async fn save(&self, record: &RunRecord) -> Result<(), RunError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| lock_poisoned("run store"))?;
        records.insert(record.id.as_str().to_string(), record.clone());
        Ok(())
    }

    async fn append_event(&self, id: &RunId, event: &Event) -> Result<(), RunError> {
        let mut events = self.events.lock().map_err(|_| lock_poisoned("run store"))?;
        events
            .entry(id.as_str().to_string())
            .or_default()
            .push(event.clone());
        Ok(())
    }

    async fn list(&self) -> Result<Vec<RunId>, RunError> {
        let records = self
            .records
            .lock()
            .map_err(|_| lock_poisoned("run store"))?;
        // Keys iterate in sorted order; each stored id was a valid RunId, so re-parsing cannot fail.
        let ids = records.keys().filter_map(|k| RunId::new(k).ok()).collect();
        Ok(ids)
    }

    async fn get(&self, id: &RunId) -> Result<Option<RunRecord>, RunError> {
        let records = self
            .records
            .lock()
            .map_err(|_| lock_poisoned("run store"))?;
        Ok(records.get(id.as_str()).cloned())
    }

    async fn prune(&self, cutoff: &Timestamp) -> Result<u32, RunError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| lock_poisoned("run store"))?;
        let mut events = self.events.lock().map_err(|_| lock_poisoned("run store"))?;
        // RFC 3339 UTC instants sort lexically, so a string compare is a chronological compare.
        let stale: Vec<String> = records
            .iter()
            .filter(|(_, record)| record.started_at.as_str() < cutoff.as_str())
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            records.remove(id);
            events.remove(id);
        }
        Ok(stale.len() as u32)
    }

    async fn remove(&self, id: &RunId) -> Result<(), RunError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| lock_poisoned("run store"))?;
        let mut events = self.events.lock().map_err(|_| lock_poisoned("run store"))?;
        records.remove(id.as_str());
        events.remove(id.as_str());
        Ok(())
    }
}
