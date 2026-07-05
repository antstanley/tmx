//! [`FakeSourceLoader`] and [`FakeReferenceResolver`] — the in-memory source-loading fakes.
//!
//! [`FakeSourceLoader`] parses nothing: it returns a seeded JSON [`Value`] for a given path with no
//! disk read. [`FakeReferenceResolver`] maps a reference string to a seeded [`ResolvedSource`]. An
//! unseeded path or reference is a typed resolution [`RunError`], never a panic.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;
use tmx_core::RunError;
use tmx_core::ports::driven::{ReferenceResolver, ResolvedSource, SourceKind, SourceLoader};

/// A [`SourceLoader`] backed by a seeded `path -> Value` map.
#[derive(Debug, Default)]
pub struct FakeSourceLoader {
    sources: Mutex<BTreeMap<String, Value>>,
}

impl FakeSourceLoader {
    /// An empty loader: every path is unresolved until seeded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the parsed `value` a load of `path` returns (builder form).
    #[must_use]
    pub fn with_source(self, path: impl Into<String>, value: Value) -> Self {
        if let Ok(mut sources) = self.sources.lock() {
            sources.insert(path.into(), value);
        }
        self
    }
}

#[async_trait::async_trait]
impl SourceLoader for FakeSourceLoader {
    async fn load(&self, path: &str, _kind: SourceKind) -> Result<Value, RunError> {
        let sources = self.sources.lock().map_err(|_| {
            RunError::run_failure(
                "source_lock_poisoned",
                "the in-memory source map lock was poisoned",
            )
        })?;
        sources.get(path).cloned().ok_or_else(|| {
            RunError::resolution("source_not_found", format!("no source seeded at {path}"))
        })
    }
}

/// A [`ReferenceResolver`] backed by a seeded `reference -> ResolvedSource` map.
#[derive(Debug, Default)]
pub struct FakeReferenceResolver {
    refs: Mutex<BTreeMap<String, ResolvedSource>>,
}

impl FakeReferenceResolver {
    /// An empty resolver: every reference is unresolved until seeded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed the resolution of `reference` to `path` as `kind` (builder form).
    #[must_use]
    pub fn with_reference(
        self,
        reference: impl Into<String>,
        path: impl Into<String>,
        kind: SourceKind,
    ) -> Self {
        if let Ok(mut refs) = self.refs.lock() {
            refs.insert(
                reference.into(),
                ResolvedSource {
                    path: path.into(),
                    kind,
                },
            );
        }
        self
    }
}

#[async_trait::async_trait]
impl ReferenceResolver for FakeReferenceResolver {
    async fn resolve(&self, reference: &str) -> Result<ResolvedSource, RunError> {
        let refs = self.refs.lock().map_err(|_| {
            RunError::run_failure(
                "ref_lock_poisoned",
                "the in-memory reference map lock was poisoned",
            )
        })?;
        refs.get(reference).cloned().ok_or_else(|| {
            RunError::resolution(
                "reference_not_found",
                format!("no reference seeded for {reference}"),
            )
        })
    }
}
