//! [`BuiltinSecretResolver`] — the built-in [`SecretResolver`] adapter (`env` / `file` / provider seam).
//!
//! Resolves a [`SecretSource`] to a value the Masker then registers as sensitive (06 §Secret
//! resolution). Three source shapes, tried in a fixed precedence:
//!
//! - [`env`](SecretSource::env) — a host environment variable (the task-17 minimal path);
//! - [`file`](SecretSource::file) — a path whose (bounded, UTF-8) contents are the value;
//! - [`provider`](SecretSource::provider) — a named backend (`aws-sm`/`gcp-sm`/`vault`/…) dispatched
//!   through the [`SecretProvider`] trait seam.
//!
//! The provider set is left **open**: v0 ships `env` and `file` built in plus the seam, and no
//! concrete provider backend — so a `provider` source with no matching backend registered is an
//! explicit typed error (`secret_provider_unavailable`), never a silently-empty secret. Every failure
//! path (an unset env var, an unreadable/oversized/non-UTF-8 file, an unknown provider, an empty
//! source) is a typed [`RunError`], never a panic and never an empty string.

use std::io::Read;
use std::sync::Arc;

use tmx_core::RunError;
use tmx_core::ports::driven::SecretResolver;
use tmx_schema::SecretSource;
use tmx_schema::limits::SECRET_FILE_MAX_BYTES;

/// A named secret-provider backend (`aws-sm`, `gcp-sm`, `vault`, …) — the open seam behind the
/// [`SecretResolver`]'s `provider` source.
///
/// v0 ships no concrete implementation: the trait exists so a future backend is *a new adapter behind
/// this seam*, never a change to the resolver or the core (06 §Decisions — "a trait seam with
/// `env`/`file` built in; `aws-sm`/`vault`/… ship as adapters"). A backend answers to the
/// [`name`](SecretProvider::name) a `secretSource.provider` names, and reads the value from the full
/// [`SecretSource`] (its `key` plus any provider-specific `extra` keys).
#[async_trait::async_trait]
pub trait SecretProvider: Send + Sync {
    /// The `provider` name this backend answers to (matched case-sensitively against
    /// `secretSource.provider`).
    fn name(&self) -> &str;

    /// Fetch the secret described by `source` (its [`key`](SecretSource::key) plus any
    /// provider-specific [`extra`](SecretSource::extra) keys). A missing/denied key is a typed
    /// [`RunError`].
    ///
    /// # Errors
    ///
    /// Returns a resolution [`RunError`] when the backend cannot produce the value.
    async fn fetch(&self, source: &SecretSource) -> Result<String, RunError>;
}

/// A [`SecretResolver`] that reads `env` and `file` secrets directly and dispatches `provider`
/// secrets through registered [`SecretProvider`] backends.
///
/// Constructed with no backends ([`new`](Self::new)); a build wires concrete providers with
/// [`with_provider`](Self::with_provider). Stateless with respect to `env`/`file`: it reads the live
/// process environment / filesystem on each call.
#[derive(Clone, Default)]
pub struct BuiltinSecretResolver {
    /// Registered provider backends, matched by [`SecretProvider::name`]. Empty in the v0 build.
    providers: Vec<Arc<dyn SecretProvider>>,
    /// The ceiling, in bytes, applied to a `file`-sourced secret read.
    file_cap_bytes: u64,
}

impl std::fmt::Debug for BuiltinSecretResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn SecretProvider` is not `Debug`; report the count and cap, never a backend's innards.
        f.debug_struct("BuiltinSecretResolver")
            .field("providers", &self.providers.len())
            .field("file_cap_bytes", &self.file_cap_bytes)
            .finish()
    }
}

impl BuiltinSecretResolver {
    /// A fresh resolver with `env`/`file` support and no provider backends. The `file` read is
    /// bounded by [`SECRET_FILE_MAX_BYTES`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            providers: Vec::new(),
            file_cap_bytes: SECRET_FILE_MAX_BYTES,
        }
    }

    /// Register a named [`SecretProvider`] backend, returning the extended resolver. Resolution scans
    /// the backends in registration order and dispatches to the first whose
    /// [`name`](SecretProvider::name) matches, so when two backends share a name the
    /// first-registered one wins and a later duplicate is never reached.
    #[must_use]
    pub fn with_provider(mut self, provider: Arc<dyn SecretProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    /// Read a `file`-sourced secret: the file's bytes, bounded by
    /// [`file_cap_bytes`](Self::file_cap_bytes) and required to be valid UTF-8. A single trailing
    /// newline (`\n` or `\r\n`) is stripped so a token written by `echo`/an editor resolves to the
    /// token itself, not the token plus a newline.
    fn read_file(&self, path: &str) -> Result<String, RunError> {
        let file = std::fs::File::open(path).map_err(|source| {
            RunError::resolution(
                "secret_file_unreadable",
                format!("file secret `{path}` cannot be opened: {source}"),
            )
        })?;
        // Read at most cap + 1 bytes so an over-cap file is detected without buffering it whole.
        let cap = self.file_cap_bytes;
        let mut buf = Vec::new();
        file.take(cap.saturating_add(1))
            .read_to_end(&mut buf)
            .map_err(|source| {
                RunError::resolution(
                    "secret_file_unreadable",
                    format!("file secret `{path}` cannot be read: {source}"),
                )
            })?;
        if buf.len() as u64 > cap {
            return Err(RunError::resolution(
                "secret_file_too_large",
                format!("file secret `{path}` exceeds the {cap}-byte secret-file cap"),
            ));
        }
        let text = String::from_utf8(buf).map_err(|_| {
            RunError::resolution(
                "secret_file_not_utf8",
                format!("file secret `{path}` is not valid UTF-8"),
            )
        })?;
        let trimmed = text
            .strip_suffix('\n')
            .map_or(text.as_str(), |s| s.strip_suffix('\r').unwrap_or(s));
        reject_empty(
            trimmed.to_string(),
            format!("file secret `{path}` resolved to an empty value"),
        )
    }

    /// Dispatch a `provider`-sourced secret to the registered backend whose name matches. With no
    /// matching backend (the v0 default), a typed `secret_provider_unavailable` error — never a
    /// silently-empty value.
    async fn resolve_provider(
        &self,
        provider: &str,
        source: &SecretSource,
    ) -> Result<String, RunError> {
        let backend = self.providers.iter().find(|p| p.name() == provider);
        match backend {
            // A backend that answers `Ok("")` would defeat masking exactly as an empty env/file
            // secret does; hold it to the same non-empty contract as the built-in shapes.
            Some(backend) => reject_empty(
                backend.fetch(source).await?,
                format!("provider secret `{provider}` resolved to an empty value"),
            ),
            None => Err(RunError::resolution(
                "secret_provider_unavailable",
                format!(
                    "no secret-provider backend is registered for provider `{provider}` \
                     (env/file are built in; providers ship as adapters)"
                ),
            )),
        }
    }
}

/// Read an `env`-sourced secret from the live process environment. An unset var is a typed error, and
/// so is a set-but-empty one: a resolved secret is never the empty string (see the module doc), because
/// the Masker cannot register an empty value as sensitive and downstream masking would be defeated.
fn read_env(var: &str) -> Result<String, RunError> {
    let value = std::env::var(var).map_err(|_| {
        RunError::resolution(
            "secret_not_found",
            format!("environment secret `{var}` is not set"),
        )
    })?;
    reject_empty(
        value,
        format!("environment secret `{var}` resolved to an empty value"),
    )
}

/// Guard a resolved secret against the empty string. The Masker skips empty registrations by design,
/// so an empty resolved value would slip through unmasked; a typed `secret_value_empty` error stops the
/// run instead of letting an unmaskable secret through (or panicking downstream).
fn reject_empty(value: String, message: String) -> Result<String, RunError> {
    if value.is_empty() {
        return Err(RunError::resolution("secret_value_empty", message));
    }
    Ok(value)
}

#[async_trait::async_trait]
impl SecretResolver for BuiltinSecretResolver {
    async fn resolve(&self, source: &SecretSource) -> Result<String, RunError> {
        // Fixed precedence: env → file → provider. The schema's `secretSource` names at most one
        // shape in practice; the precedence makes a malformed multi-key source deterministic rather
        // than order-dependent.
        if let Some(var) = &source.env {
            return read_env(var);
        }
        if let Some(path) = &source.file {
            return self.read_file(path);
        }
        if let Some(provider) = &source.provider {
            return self.resolve_provider(provider, source).await;
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
    use indexmap::IndexMap;
    use std::future::Future;
    use std::pin::pin;
    use std::sync::atomic::{AtomicBool, Ordering};
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
            extra: IndexMap::new(),
        }
    }

    fn file_source(path: &str) -> SecretSource {
        SecretSource {
            env: None,
            file: Some(path.to_string()),
            provider: None,
            key: None,
            extra: IndexMap::new(),
        }
    }

    fn provider_source(provider: &str, key: &str) -> SecretSource {
        SecretSource {
            env: None,
            file: None,
            provider: Some(provider.to_string()),
            key: Some(key.to_string()),
            extra: IndexMap::new(),
        }
    }

    /// A unique scratch path under the OS temp dir for one file-secret test.
    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tmx-secret-{tag}-{}-{}",
            std::process::id(),
            tag.len()
        ))
    }

    #[test]
    fn resolves_a_present_env_secret() {
        // A present process env var is resolved to its exact value. `PATH` is set in every shell the
        // test suite runs under; the crate forbids `unsafe`, so the environment is read, never
        // mutated (`std::env::set_var` is `unsafe` on this edition).
        let Ok(expected) = std::env::var("PATH") else {
            return;
        };
        let resolver = BuiltinSecretResolver::new();
        let value =
            block_on_ready(resolver.resolve(&env_source("PATH"))).expect("a present var resolves");
        assert_eq!(value, expected, "the exact env value is returned verbatim");
        assert!(
            !value.is_empty(),
            "a present PATH resolves to a non-empty value"
        );
    }

    #[test]
    fn resolves_a_file_secret_and_strips_a_trailing_newline() {
        let path = temp_path("read");
        // The value carries an interior newline (kept) and a trailing one (stripped).
        std::fs::write(&path, "line-one\nline-two\n").expect("write the secret file");
        let resolver = BuiltinSecretResolver::new();
        let value = block_on_ready(resolver.resolve(&file_source(&path.to_string_lossy())))
            .expect("a present file resolves");
        assert_eq!(
            value, "line-one\nline-two",
            "the file's bytes are the value, with a single trailing newline stripped"
        );
        assert!(
            value.ends_with("two"),
            "the interior content is preserved verbatim"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn resolves_a_file_secret_with_no_trailing_newline_verbatim() {
        let path = temp_path("verbatim");
        std::fs::write(&path, "no-newline-token").expect("write the secret file");
        let resolver = BuiltinSecretResolver::new();
        let value = block_on_ready(resolver.resolve(&file_source(&path.to_string_lossy())))
            .expect("a present file resolves");
        assert_eq!(
            value, "no-newline-token",
            "a file with no trailing newline resolves to its exact contents"
        );
        assert_eq!(value.len(), 16, "no bytes are added or removed");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn an_empty_file_secret_is_a_typed_error_not_an_empty_value() {
        // A file whose only content is a trailing newline strips to "" — an empty resolved secret the
        // Masker cannot register. The resolver must reject it typed, never hand back "".
        for (tag, contents) in [
            ("empty-bytes", ""),
            ("only-newline", "\n"),
            ("only-crlf", "\r\n"),
        ] {
            let path = temp_path(tag);
            std::fs::write(&path, contents).expect("write the secret file");
            let resolver = BuiltinSecretResolver::new();
            let err = block_on_ready(resolver.resolve(&file_source(&path.to_string_lossy())))
                .expect_err("an empty file secret is an error");
            assert_eq!(
                err.code, "secret_value_empty",
                "an empty file secret names its own code, got {:?}",
                err.code
            );
            assert_eq!(
                err.category,
                tmx_core::ErrorCategory::Resolution,
                "an empty file secret is a resolution failure"
            );
            std::fs::remove_file(&path).ok();
        }
    }

    #[test]
    fn an_empty_env_secret_is_a_typed_error_not_an_empty_value() {
        // `read_env` guards a set-but-empty var directly; asserting on the guard avoids mutating the
        // process environment (this crate forbids `unsafe`, and `set_var` is `unsafe` on this edition).
        let err = read_env_value_for_test("").expect_err("an empty env value is an error");
        assert_eq!(
            err.code, "secret_value_empty",
            "an empty env secret names its own code, got {:?}",
            err.code
        );
        assert_eq!(
            err.category,
            tmx_core::ErrorCategory::Resolution,
            "an empty env secret is a resolution failure"
        );
        // A non-empty value passes the same guard through verbatim.
        let ok = read_env_value_for_test("present").expect("a non-empty env value resolves");
        assert_eq!(ok, "present", "a non-empty env value is returned verbatim");
    }

    /// Exercise the empty-value guard `read_env` applies, without touching the process environment.
    fn read_env_value_for_test(value: &str) -> Result<String, RunError> {
        reject_empty(
            value.to_string(),
            "environment secret `X` resolved to an empty value".to_string(),
        )
    }

    #[test]
    fn a_missing_file_is_a_typed_error_not_a_panic() {
        let resolver = BuiltinSecretResolver::new();
        let err = block_on_ready(resolver.resolve(&file_source("/no/such/tmx/secret/path")))
            .expect_err("a missing file is an error");
        assert_eq!(
            err.code, "secret_file_unreadable",
            "a missing file names its own code"
        );
        assert_eq!(
            err.category,
            tmx_core::ErrorCategory::Resolution,
            "a missing file secret is a resolution failure"
        );
    }

    #[test]
    fn an_oversized_file_is_rejected_by_the_cap() {
        let path = temp_path("toobig");
        std::fs::write(&path, "0123456789").expect("write the secret file");
        // A resolver with a tiny cap rejects the 10-byte file rather than buffering it.
        let resolver = BuiltinSecretResolver {
            providers: Vec::new(),
            file_cap_bytes: 4,
        };
        let err = block_on_ready(resolver.resolve(&file_source(&path.to_string_lossy())))
            .expect_err("an over-cap file is rejected");
        assert_eq!(
            err.code, "secret_file_too_large",
            "an over-cap file names the cap code"
        );
        assert!(
            err.message.contains('4'),
            "the error names the cap it exceeded, got {:?}",
            err.message
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_non_utf8_file_is_a_typed_error() {
        let path = temp_path("binary");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("write the secret file");
        let resolver = BuiltinSecretResolver::new();
        let err = block_on_ready(resolver.resolve(&file_source(&path.to_string_lossy())))
            .expect_err("a non-UTF-8 file is an error");
        assert_eq!(
            err.code, "secret_file_not_utf8",
            "a non-UTF-8 file names its own code"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_env_var_and_empty_source_are_typed_errors() {
        let resolver = BuiltinSecretResolver::new();
        let missing = block_on_ready(resolver.resolve(&env_source("TMX_DEFINITELY_UNSET_XYZ")))
            .expect_err("an unset env var is an error");
        assert_eq!(missing.code, "secret_not_found", "names the missing secret");
        assert_eq!(
            missing.category,
            tmx_core::ErrorCategory::Resolution,
            "a missing secret is a resolution failure"
        );

        let empty = SecretSource {
            env: None,
            file: None,
            provider: None,
            key: None,
            extra: IndexMap::new(),
        };
        let empty_err =
            block_on_ready(resolver.resolve(&empty)).expect_err("an empty source is an error");
        assert_eq!(
            empty_err.code, "secret_source_empty",
            "names the empty source"
        );
    }

    #[test]
    fn a_provider_source_with_no_registered_backend_is_unavailable() {
        // The seam compiles and is reachable with no concrete backend: a `provider` source resolves
        // to a typed `secret_provider_unavailable`, never a panic and never an empty value.
        let resolver = BuiltinSecretResolver::new();
        let err = block_on_ready(resolver.resolve(&provider_source("aws-sm", "db/password")))
            .expect_err("no backend is registered in the v0 build");
        assert_eq!(
            err.code, "secret_provider_unavailable",
            "an unbacked provider names its own code"
        );
        assert!(
            err.message.contains("aws-sm"),
            "the error names the requested provider, got {:?}",
            err.message
        );
    }

    /// A test-only [`SecretProvider`] that answers to one name and records whether it was asked.
    struct RecordingProvider {
        name: String,
        value: String,
        called: AtomicBool,
    }

    #[async_trait::async_trait]
    impl SecretProvider for RecordingProvider {
        fn name(&self) -> &str {
            &self.name
        }
        async fn fetch(&self, source: &SecretSource) -> Result<String, RunError> {
            self.called.store(true, Ordering::SeqCst);
            assert!(
                source.key.is_some(),
                "the resolver hands the backend the full source, key included"
            );
            Ok(self.value.clone())
        }
    }

    #[test]
    fn a_registered_provider_backend_resolves_its_source() {
        let backend = Arc::new(RecordingProvider {
            name: "vault".to_string(),
            value: "provided-secret".to_string(),
            called: AtomicBool::new(false),
        });
        let resolver = BuiltinSecretResolver::new().with_provider(backend.clone());
        let value = block_on_ready(resolver.resolve(&provider_source("vault", "kv/token")))
            .expect("the registered backend resolves");
        assert_eq!(value, "provided-secret", "the backend's value is returned");
        assert!(
            backend.called.load(Ordering::SeqCst),
            "the matching backend was dispatched to"
        );

        // Negative space: a provider name no backend answers to is still unavailable, even with one
        // other backend registered — dispatch is keyed strictly by name.
        let other = block_on_ready(resolver.resolve(&provider_source("aws-sm", "kv/token")))
            .expect_err("an unregistered provider name has no backend");
        assert_eq!(
            other.code, "secret_provider_unavailable",
            "an unmatched provider name is unavailable"
        );
    }

    #[test]
    fn a_provider_backend_returning_an_empty_value_is_a_typed_error_not_an_empty_value() {
        // Negative space / defence in depth: a registered backend that answers `Ok("")` must not slip
        // an unmaskable empty secret downstream. The resolver holds it to the same non-empty contract
        // as env/file, surfacing a typed `secret_value_empty`, never `Ok("")`.
        let backend = Arc::new(RecordingProvider {
            name: "vault".to_string(),
            value: String::new(),
            called: AtomicBool::new(false),
        });
        let resolver = BuiltinSecretResolver::new().with_provider(backend.clone());
        let err = block_on_ready(resolver.resolve(&provider_source("vault", "kv/token")))
            .expect_err("an empty provider value is an error");
        assert_eq!(
            err.code, "secret_value_empty",
            "an empty provider value names the empty-secret code, got {:?}",
            err.code
        );
        assert_eq!(
            err.category,
            tmx_core::ErrorCategory::Resolution,
            "an empty provider value is a resolution failure"
        );
        assert!(
            backend.called.load(Ordering::SeqCst),
            "the backend was actually dispatched to before the guard rejected its value"
        );
    }
}
