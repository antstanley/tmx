//! [`FakeEnvironmentProvider`] — the scripted [`EnvironmentProvider`] fake.
//!
//! Materialises no real substrate: it returns a fixed JSON output for each provider lifecycle
//! [`ProviderMethod`] (`bootstrap`/`deploy`/`clean`/`destroy`) and records the methods invoked, so a
//! test drives the `environment` block deterministically and asserts on the exact lifecycle calls.

use std::sync::Mutex;

use serde_json::json;
use tmx_core::Milliseconds;
use tmx_core::RunError;
use tmx_core::ports::driven::{EnvironmentProvider, ProviderMethod, ProviderOutcome};
use tmx_schema::Environment;

/// An [`EnvironmentProvider`] that returns a fixed outcome per method and records the calls.
#[derive(Debug, Default)]
pub struct FakeEnvironmentProvider {
    calls: Mutex<Vec<ProviderMethod>>,
}

impl FakeEnvironmentProvider {
    /// A provider whose every lifecycle method succeeds with a fixed `{ "ok": true }` output.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The lifecycle methods this provider was asked to invoke, in call order.
    #[must_use]
    pub fn calls(&self) -> Vec<ProviderMethod> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl EnvironmentProvider for FakeEnvironmentProvider {
    async fn invoke(
        &self,
        method: ProviderMethod,
        _environment: &Environment,
    ) -> Result<ProviderOutcome, RunError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(method);
        }
        Ok(ProviderOutcome {
            method,
            output: json!({ "ok": true }),
            ms: Milliseconds(0),
        })
    }
}
