//! [`FakeSchemaValidator`] — the scripted [`SchemaValidator`] fake.
//!
//! Validates nothing structurally: it returns a seeded, fixed set of [`Diagnostic`]s (empty by
//! default, i.e. "valid") for both artifact and `produces` validation. A test seeds diagnostics to
//! drive the invalid path deterministically. Sync, mirroring the port — validation has no effecting
//! boundary.

use serde_json::Value;
use tmx_core::Diagnostic;
use tmx_core::RunError;
use tmx_core::ports::driven::{ArtifactKind, SchemaValidator};

/// A [`SchemaValidator`] that returns a fixed, seeded diagnostic list.
///
/// The empty default reports every instance as valid; seeding diagnostics drives the invalid path.
#[derive(Debug, Default, Clone)]
pub struct FakeSchemaValidator {
    diagnostics: Vec<Diagnostic>,
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
        Ok(self.diagnostics.clone())
    }
}
