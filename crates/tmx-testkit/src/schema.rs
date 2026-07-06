//! [`FakeSchemaValidator`] — the scripted [`SchemaValidator`] fake.
//!
//! Validates nothing structurally: it returns a seeded, fixed set of [`Diagnostic`]s (empty by
//! default, i.e. "valid") for both artifact and `produces` validation. A test seeds diagnostics to
//! drive the invalid path deterministically. Sync, mirroring the port — validation has no effecting
//! boundary. It also counts `validate_produces` calls, so a test can prove the runtime `produces`
//! check was reached under `--check-produces` and skipped when the flag is absent.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use tmx_core::Diagnostic;
use tmx_core::RunError;
use tmx_core::ports::driven::{ArtifactKind, SchemaValidator};

/// A [`SchemaValidator`] that returns a fixed, seeded diagnostic list.
///
/// The empty default reports everything valid; seeding diagnostics drives the invalid path. Every
/// `validate_produces` call is counted (via a shared counter that survives cloning), so a test can
/// assert whether the runtime `produces` check ran.
#[derive(Debug, Default, Clone)]
pub struct FakeSchemaValidator {
    diagnostics: Vec<Diagnostic>,
    produces_calls: Arc<AtomicUsize>,
}

impl FakeSchemaValidator {
    /// A validator that reports everything valid (no diagnostics).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a diagnostic returned by every `validate`/`validate_produces` call (builder form).
    #[must_use]
    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostics.push(diagnostic);
        self
    }

    /// The number of `validate_produces` calls made so far — the seam a test uses to prove the runtime
    /// `produces` check was (or was not) reached.
    #[must_use]
    pub fn produces_call_count(&self) -> usize {
        self.produces_calls.load(Ordering::SeqCst)
    }
}

impl SchemaValidator for FakeSchemaValidator {
    fn validate(
        &self,
        _instance: &Value,
        _kind: ArtifactKind,
    ) -> Result<Vec<Diagnostic>, RunError> {
        Ok(self.diagnostics.clone())
    }

    fn validate_produces(
        &self,
        _output: &Value,
        _schema: &Value,
    ) -> Result<Vec<Diagnostic>, RunError> {
        self.produces_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.diagnostics.clone())
    }
}
