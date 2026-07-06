//! [`EnvSecretResolver`] — the minimal `env` [`SecretResolver`] adapter.
//!
//! Resolves an [`env`](SecretSource::env)-sourced secret from the process environment, so a requested
//! secret resolves and the end-to-end masking guarantee is demonstrable (a task that echoes the
//! secret has it redacted out of stdout). The `file` source and the named-provider seam are task 24;
//! here they are explicit typed errors, never a silent empty value. An unset env var is a typed
//! resolution error, not a panic.

use tmx_core::RunError;
use tmx_core::ports::driven::SecretResolver;
use tmx_schema::SecretSource;

/// A [`SecretResolver`] that reads `env`-sourced secrets from the process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvSecretResolver;

impl EnvSecretResolver {
    /// A fresh resolver. Stateless: it reads the live process environment on each call.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl SecretResolver for EnvSecretResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<String, RunError> {
        if let Some(var) = &source.env {
            return std::env::var(var).map_err(|_| {
                RunError::resolution(
                    "secret_not_found",
                    format!("environment secret `{var}` is not set"),
                )
            });
        }
        if source.file.is_some() || source.provider.is_some() {
            // The `file` source and the named-provider seam are task 24; reject them explicitly so a
            // flow that needs them fails with a typed error rather than a silently-empty secret.
            return Err(RunError::resolution(
                "secret_source_unsupported",
                "only `env` secret sources resolve in this build (file/provider arrive in task 24)",
            ));
        }
        Err(RunError::resolution(
            "secret_source_empty",
            "the secret source names neither env, file, nor provider/key",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    fn env_source(var: &str) -> SecretSource {
        SecretSource {
            env: Some(var.to_string()),
            file: None,
            provider: None,
            key: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn resolves_a_present_env_secret() {
        // A present process env var is resolved to its exact value. `PATH` is set in every shell the
        // test suite runs under; mutating the environment is avoided because the crate forbids
        // `unsafe`, and `std::env::set_var` is `unsafe` on this edition.
        let Ok(expected) = std::env::var("PATH") else {
            // Without PATH there is nothing present to resolve; skip rather than assert a fixture.
            return;
        };
        let resolver = EnvSecretResolver::new();
        let value =
            block_on_ready(resolver.resolve(&env_source("PATH"))).expect("a present var resolves");
        assert_eq!(value, expected, "the exact env value is returned verbatim");
        assert!(
            !value.is_empty(),
            "a present PATH resolves to a non-empty value"
        );
    }

    #[test]
    fn missing_env_var_and_unsupported_sources_are_typed_errors() {
        let resolver = EnvSecretResolver::new();
        // Negative space: an unset env var is a typed resolution error, not an empty string.
        let missing = block_on_ready(resolver.resolve(&env_source("TMX_DEFINITELY_UNSET_XYZ")))
            .expect_err("an unset env var is an error");
        assert_eq!(missing.code, "secret_not_found", "names the missing secret");
        assert_eq!(
            missing.category,
            tmx_core::ErrorCategory::Resolution,
            "a missing secret is a resolution failure"
        );

        // A file source is explicitly unsupported in this build (task 24), not silently empty.
        let file_source = SecretSource {
            env: None,
            file: Some("/run/secrets/token".to_string()),
            provider: None,
            key: None,
            extra: Default::default(),
        };
        let unsupported = block_on_ready(resolver.resolve(&file_source))
            .expect_err("a file source is unsupported here");
        assert_eq!(
            unsupported.code, "secret_source_unsupported",
            "a file source is rejected with its own code"
        );

        // An empty source names nothing to resolve.
        let empty = SecretSource {
            env: None,
            file: None,
            provider: None,
            key: None,
            extra: Default::default(),
        };
        let empty_err =
            block_on_ready(resolver.resolve(&empty)).expect_err("an empty source is an error");
        assert_eq!(
            empty_err.code, "secret_source_empty",
            "names the empty source"
        );
    }
}
