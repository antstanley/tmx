//! [`FakeSecretResolver`] — the in-memory [`SecretResolver`] fake.
//!
//! Resolves a [`SecretSource`] against a seeded `name -> value` map with no real env/file/provider
//! lookup. The lookup name is the source's `env` var, else its `provider`+`key` pair. An unseeded
//! source is a typed resolution [`RunError`] — the negative space a missing secret would raise.

use std::collections::BTreeMap;
use std::sync::Mutex;

use tmx_core::RunError;
use tmx_core::ports::driven::SecretResolver;
use tmx_schema::SecretSource;

/// A [`SecretResolver`] backed by a seeded map of secret names to values.
#[derive(Debug, Default)]
pub struct FakeSecretResolver {
    secrets: Mutex<BTreeMap<String, String>>,
}

impl FakeSecretResolver {
    /// An empty resolver: every source is unresolved until seeded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a secret `name` (an env var name or a `provider/key` string) with `value`.
    #[must_use]
    pub fn with_secret(self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let Ok(mut secrets) = self.secrets.lock() {
            secrets.insert(name.into(), value.into());
        }
        self
    }
}

/// The lookup name a [`SecretSource`] resolves under: its `env` var, else `provider/key`.
fn lookup_name(source: &SecretSource) -> Option<String> {
    if let Some(env) = &source.env {
        return Some(env.clone());
    }
    match (&source.provider, &source.key) {
        (Some(provider), Some(key)) => Some(format!("{provider}/{key}")),
        _ => source.file.clone(),
    }
}

#[async_trait::async_trait]
impl SecretResolver for FakeSecretResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<String, RunError> {
        let name = lookup_name(source).ok_or_else(|| {
            RunError::resolution(
                "secret_source_empty",
                "the secret source names neither env, file, nor provider/key",
            )
        })?;
        let secrets = self.secrets.lock().map_err(|_| {
            RunError::run_failure(
                "secret_lock_poisoned",
                "the in-memory secret map lock was poisoned",
            )
        })?;
        secrets.get(&name).cloned().ok_or_else(|| {
            RunError::resolution("secret_not_found", format!("no secret seeded for {name}"))
        })
    }
}
