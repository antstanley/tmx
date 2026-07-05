//! The `Masker` — the domain-policy secret redactor enforced at the port boundary.
//!
//! Masking is a **domain policy**, not an adapter concern: every value leaving the core through an
//! output port (`EventSink` payloads, the final-state serialisation, `RunStore` writes, log lines)
//! is scrubbed of every resolved secret, including within nested JSON, so no adapter can leak a
//! secret regardless of its own correctness
//! ([04 §Secrets & masking](../../../.specs/04-execution-engine.md#secrets--masking);
//! [08 §Masking at the boundary](../../../.specs/08-errors-and-observability.md#masking-at-the-boundary)).
//!
//! It is a **pure, sync** unit: a [`Masker`] holds a registry of sensitive values and the redaction
//! is plain value scanning — no port, no `await`, no I/O. It builds on the Task 04 [`serde_json::Value`]
//! model and the Task 02 [`tmx_schema::limits::MASK_SCAN_LEN_MIN_BYTES`] floor.
//!
//! ## Value-based redaction, with a scan floor
//!
//! Redaction is **value-based** in v0: every registered value is scanned for across an emitted
//! payload. A registered value whose byte length is `>= MASK_SCAN_LEN_MIN_BYTES` (default 6) is
//! redacted wherever it appears **as a substring** (so an echoed secret embedded in a larger string
//! is caught); a value **shorter** than the floor is redacted only on an **exact whole-value match**
//! — a 4-byte secret must not clobber every unrelated string that happens to contain those bytes.
//!
//! ## Paired boundary assertions (Tiger Style negative space)
//!
//! The guarantee is enforced along **two independent paths**, so neither alone is load-bearing:
//!
//! - [`Masker::assert_ready`] — the *registry-populated* side: before any output port runs, the
//!   runner asserts every resolved secret is registered (else redaction would have nothing to scan
//!   for).
//! - [`Masker::assert_routed`] — the *output-port* side: each port asserts the payload it is about
//!   to emit was produced by *this* Masker (a [`Masked`] token whose origin matches). A payload that
//!   bypassed the Masker fails this assertion, so masking cannot be sidestepped by adding a sink.
//!
//! A [`Masked<T>`] payload can only be minted by [`Masker::redact_value`] / [`Masker::redact_line`],
//! so an output port that accepts `Masked<T>` is statically prevented from emitting un-scrubbed data;
//! the runtime `assert_routed` is the paired, defence-in-depth check on top of that type guarantee.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tmx_schema::limits::MASK_SCAN_LEN_MIN_BYTES;

/// The token substituted for every redacted occurrence of a sensitive value.
///
/// A fixed marker (not a numeric bound, so it lives here rather than in `tmx-schema::limits`); it is
/// deliberately not derived from the secret so its length leaks nothing about what it replaced.
const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

/// Monotonic source of per-`Masker` identities, so [`Masked`] payloads can be tied to the exact
/// Masker that produced them. Starts at 1; a zero id is never handed out (it marks "uninitialised").
static NEXT_MASKER_ID: AtomicU64 = AtomicU64::new(1);

/// A payload proven to have been routed through a [`Masker`].
///
/// Output ports accept `Masked<T>` rather than a raw `T`, so a sink cannot emit un-scrubbed data by
/// construction; [`Masked::origin`] additionally records *which* Masker minted it, letting a port
/// assert the payload belongs to this run's Masker via [`Masker::assert_routed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked<T> {
    /// The redacted payload.
    value: T,
    /// The id of the [`Masker`] that produced this payload (never 0).
    origin: u64,
}

impl<T> Masked<T> {
    /// Borrow the redacted payload.
    #[must_use]
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Consume the token, yielding the redacted payload.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.value
    }

    /// The id of the Masker that produced this payload.
    #[must_use]
    pub fn origin(&self) -> u64 {
        self.origin
    }
}

/// The domain-policy secret redactor: a registry of sensitive values plus a value scanner.
///
/// Registered values are partitioned once, at registration, into a substring-scanned bucket
/// (`>= MASK_SCAN_LEN_MIN_BYTES`) kept sorted longest-first for overlap-safe redaction, and an
/// exact-match-only bucket (below the floor). The hot path — [`redact_value`](Self::redact_value)
/// on every emission — then does no partitioning or sorting, and borrows any subtree it does not
/// change.
#[derive(Debug)]
pub struct Masker {
    /// Sensitive values with byte length `>= MASK_SCAN_LEN_MIN_BYTES`, redacted by substring scan.
    /// Kept sorted longest-first so a value that contains a shorter registered value is scrubbed
    /// first, leaving no partial leak.
    substring: Vec<String>,
    /// Sensitive values shorter than the floor, redacted only on an exact whole-value match.
    exact: Vec<String>,
    /// This Masker's identity, stamped into every [`Masked`] payload it mints (never 0).
    id: u64,
}

impl Default for Masker {
    fn default() -> Self {
        Self::new()
    }
}

impl Masker {
    /// Create an empty Masker with a fresh, non-zero identity.
    #[must_use]
    pub fn new() -> Self {
        let id = NEXT_MASKER_ID.fetch_add(1, Ordering::Relaxed);
        // Postcondition: a fresh Masker has a usable (non-zero) identity and an empty registry.
        assert_ne!(id, 0, "masker id counter must never hand out zero");
        let masker = Self {
            substring: Vec::new(),
            exact: Vec::new(),
            id,
        };
        assert!(masker.is_empty(), "a fresh masker starts with no secrets");
        masker
    }

    /// Record a resolved secret value as sensitive.
    ///
    /// Idempotent per value. An **empty** value is ignored (it would otherwise match everywhere and
    /// is never a real secret — this is user/config data, not a programmer error, so it is skipped
    /// rather than asserted). The value is bucketed by the [`MASK_SCAN_LEN_MIN_BYTES`] floor and the
    /// substring bucket is re-sorted longest-first.
    pub fn register(&mut self, secret: impl Into<String>) {
        let secret = secret.into();
        if secret.is_empty() {
            return;
        }
        let floor = usize::try_from(MASK_SCAN_LEN_MIN_BYTES).unwrap_or(usize::MAX);
        // Precondition on the split: the floor is a positive byte length (guaranteed by the
        // compile-time `MASK_SCAN_LEN_MIN_BYTES >= 1` assertion in tmx-schema::limits).
        assert!(floor >= 1, "the mask-scan floor must be at least one byte");
        let bucket = if secret.len() >= floor {
            &mut self.substring
        } else {
            &mut self.exact
        };
        if bucket.iter().all(|existing| existing != &secret) {
            bucket.push(secret.clone());
        }
        // Longest-first keeps overlapping secrets (one a substring of another) from leaving a
        // partial leak: the longer match is consumed before the shorter one is tried.
        self.substring
            .sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
        // Postconditions: the value is now registered, and the registry is non-empty.
        assert!(
            self.contains(&secret),
            "a registered secret must be findable afterwards"
        );
        assert!(!self.is_empty(), "the registry is non-empty after register");
    }

    /// Whether `value` is registered as sensitive in either bucket.
    #[must_use]
    pub fn contains(&self, value: &str) -> bool {
        self.substring.iter().any(|s| s == value) || self.exact.iter().any(|s| s == value)
    }

    /// Whether no sensitive value has been registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.substring.is_empty() && self.exact.is_empty()
    }

    /// The number of distinct registered sensitive values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.substring.len() + self.exact.len()
    }

    /// This Masker's identity (never 0).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Redact a JSON value, returning a [`Masked`] payload an output port may emit.
    ///
    /// Every string leaf at any nesting depth is scrubbed; an unchanged subtree is borrowed
    /// (`Cow::Borrowed`) so a payload with no secrets allocates nothing.
    #[must_use]
    pub fn redact_value<'a>(&self, value: &'a Value) -> Masked<Cow<'a, Value>> {
        let scrubbed = self.scrub_value(value);
        // Postcondition (negative space): no substring-scanned secret survives in the output. Run in
        // debug only — it re-serialises the payload, which the hot path cannot afford in release.
        debug_assert!(
            self.no_substring_secret_leaked_in_value(&scrubbed),
            "a substring secret survived value redaction"
        );
        // Postcondition: the token carries this Masker's non-zero identity.
        assert_ne!(
            self.id, 0,
            "masker id must be initialised before minting a token"
        );
        Masked {
            value: scrubbed,
            origin: self.id,
        }
    }

    /// Redact a plain (non-JSON) line — a log line — returning a [`Masked`] payload.
    ///
    /// A line with no secret is borrowed unchanged.
    #[must_use]
    pub fn redact_line<'a>(&self, line: &'a str) -> Masked<Cow<'a, str>> {
        let scrubbed = self.scrub_str(line);
        debug_assert!(
            self.no_substring_secret_leaked_in_str(&scrubbed),
            "a substring secret survived line redaction"
        );
        assert_ne!(
            self.id, 0,
            "masker id must be initialised before minting a token"
        );
        Masked {
            value: scrubbed,
            origin: self.id,
        }
    }

    /// Assert the registry is populated with every resolved secret before any output port runs.
    ///
    /// The *registry-populated* half of the paired boundary assertion: aborts if a resolved secret
    /// was not registered, so a run cannot start emitting with an under-populated Masker.
    pub fn assert_ready(&self, resolved_secrets: &[&str]) {
        // A run that resolved any (non-empty) secret must have a populated registry — checked in
        // aggregate first so an entirely empty registry fails fast.
        let has_non_empty = resolved_secrets.iter().any(|s| !s.is_empty());
        assert!(
            !has_non_empty || !self.is_empty(),
            "masker registry is empty though secrets were resolved"
        );
        // And every individual resolved secret must be registered, so redaction has it to scan for.
        for secret in resolved_secrets {
            assert!(
                self.contains(secret),
                "masker registry is missing a resolved secret before output"
            );
        }
    }

    /// Assert an about-to-be-emitted payload was routed through *this* Masker.
    ///
    /// The *output-port* half of the paired boundary assertion: aborts if the payload's origin does
    /// not match this Masker's id, so a payload that bypassed redaction (or came from a foreign
    /// Masker that never registered this run's secrets) cannot be emitted.
    pub fn assert_routed<T>(&self, payload: &Masked<T>) {
        assert_ne!(self.id, 0, "masker id must be initialised");
        assert_eq!(
            payload.origin, self.id,
            "output payload was not routed through this Masker"
        );
    }

    /// Recursively scrub a JSON value, borrowing any unchanged subtree.
    fn scrub_value<'a>(&self, value: &'a Value) -> Cow<'a, Value> {
        match value {
            Value::String(text) => match self.scrub_str(text) {
                Cow::Borrowed(_) => Cow::Borrowed(value),
                Cow::Owned(redacted) => Cow::Owned(Value::String(redacted)),
            },
            // A secret re-encoded as a JSON number (a stringified secret on the way out) is still
            // caught: scan the number's textual form and, on a hit, redact it to the placeholder
            // string — a redacted value must not survive merely because its type changed.
            Value::Number(number) => {
                let rendered = number.to_string();
                match self.scrub_str(&rendered) {
                    Cow::Borrowed(_) => Cow::Borrowed(value),
                    Cow::Owned(redacted) => Cow::Owned(Value::String(redacted)),
                }
            }
            Value::Array(items) => {
                let scrubbed: Vec<Cow<'a, Value>> =
                    items.iter().map(|item| self.scrub_value(item)).collect();
                if scrubbed.iter().all(|c| matches!(c, Cow::Borrowed(_))) {
                    Cow::Borrowed(value)
                } else {
                    Cow::Owned(Value::Array(
                        scrubbed.into_iter().map(Cow::into_owned).collect(),
                    ))
                }
            }
            Value::Object(map) => {
                let scrubbed: Vec<(&String, Cow<'a, Value>)> = map
                    .iter()
                    .map(|(key, val)| (key, self.scrub_value(val)))
                    .collect();
                if scrubbed.iter().all(|(_, c)| matches!(c, Cow::Borrowed(_))) {
                    Cow::Borrowed(value)
                } else {
                    // Pre-sized buffer: the redacted object has exactly the input's key count.
                    let mut out = serde_json::Map::with_capacity(map.len());
                    for (key, val) in scrubbed {
                        out.insert(key.clone(), val.into_owned());
                    }
                    Cow::Owned(Value::Object(out))
                }
            }
            Value::Null | Value::Bool(_) => Cow::Borrowed(value),
        }
    }

    /// Scrub a single string, borrowing it unchanged when no secret occurs.
    ///
    /// Below-floor secrets redact only on an exact whole-value match; at/above-floor secrets redact
    /// every substring occurrence, longest-first.
    fn scrub_str<'a>(&self, input: &'a str) -> Cow<'a, str> {
        // Below-floor secrets: exact whole-value match only, so a short secret cannot clobber an
        // unrelated string that merely contains its bytes.
        for secret in &self.exact {
            if input == secret {
                return Cow::Owned(REDACTED_PLACEHOLDER.to_string());
            }
        }
        // At/above-floor secrets: substring scan, longest-first for overlap safety.
        let mut current: Cow<'a, str> = Cow::Borrowed(input);
        for secret in &self.substring {
            if current.contains(secret.as_str()) {
                current = Cow::Owned(current.replace(secret.as_str(), REDACTED_PLACEHOLDER));
            }
        }
        current
    }

    /// Debug postcondition: no substring-scanned secret survives anywhere in a redacted value.
    fn no_substring_secret_leaked_in_value(&self, value: &Value) -> bool {
        match value {
            Value::String(text) => self.no_substring_secret_leaked_in_str(text),
            Value::Array(items) => items
                .iter()
                .all(|item| self.no_substring_secret_leaked_in_value(item)),
            Value::Object(map) => map
                .values()
                .all(|val| self.no_substring_secret_leaked_in_value(val)),
            Value::Number(_) | Value::Null | Value::Bool(_) => true,
        }
    }

    /// Debug postcondition: no substring-scanned secret survives in a redacted string.
    fn no_substring_secret_leaked_in_str(&self, text: &str) -> bool {
        self.substring
            .iter()
            .all(|secret| !text.contains(secret.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A Masker with one long (substring-scanned) secret registered.
    fn masker_with(secret: &str) -> Masker {
        let mut masker = Masker::new();
        masker.register(secret);
        masker
    }

    // -----------------------------------------------------------------------------------------
    // O1 — nested-JSON redaction, substring for >= floor, exact-only for below-floor.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn redacts_secret_buried_in_nested_json() {
        let secret = "sk-abcdef123456"; // well above the floor
        let masker = masker_with(secret);
        let payload = json!({
            "outer": {
                "list": [
                    {"note": format!("token is {secret} keep quiet")},
                    "unrelated"
                ]
            }
        });
        let masked = masker.redact_value(&payload);
        let text = serde_json::to_string(masked.get()).expect("serialises");
        assert!(
            !text.contains(secret),
            "the buried secret must be scrubbed at depth: {text}"
        );
        assert!(
            text.contains(REDACTED_PLACEHOLDER),
            "a redaction placeholder replaces it: {text}"
        );
    }

    #[test]
    fn above_floor_secret_redacts_as_substring() {
        let secret = "hunter2secret"; // len >= floor
        assert!(secret.len() >= MASK_SCAN_LEN_MIN_BYTES as usize);
        let masker = masker_with(secret);
        let value = json!(format!("prefix-{secret}-suffix"));
        let masked = masker.redact_value(&value);
        let out = masked
            .get()
            .as_ref()
            .as_str()
            .expect("string leaf")
            .to_string();
        assert_eq!(
            out, "prefix-[REDACTED]-suffix",
            "embedded substring is redacted, surrounding text kept"
        );
        assert!(!out.contains(secret), "no residue of the secret remains");
    }

    #[test]
    fn below_floor_secret_redacts_on_exact_match_only() {
        let secret = "abcd"; // 4 bytes, below the 6-byte floor
        assert!(secret.len() < MASK_SCAN_LEN_MIN_BYTES as usize);
        let masker = masker_with(secret);

        // Exact whole-value match at a leaf is redacted.
        let exact = json!({"key": "abcd"});
        let masked_exact = masker.redact_value(&exact);
        assert_eq!(
            masked_exact.get().as_ref(),
            &json!({"key": "[REDACTED]"}),
            "a leaf equal to the short secret is redacted"
        );

        // The same bytes as a substring of unrelated text are left intact (non-clobber).
        let substr = json!({"key": "abcdef gradebook"});
        let masked_substr = masker.redact_value(&substr);
        assert_eq!(
            masked_substr.get().as_ref(),
            &json!({"key": "abcdef gradebook"}),
            "a short secret must not clobber unrelated text containing its bytes"
        );
    }

    #[test]
    fn payload_without_secrets_is_borrowed() {
        let masker = masker_with("sk-longsecret");
        let clean = json!({"a": [1, 2, "safe"], "b": "nothing here"});
        let masked = masker.redact_value(&clean);
        assert!(
            matches!(masked.get(), Cow::Borrowed(_)),
            "an unchanged payload is borrowed, not reallocated"
        );
        assert_eq!(
            masked.get().as_ref(),
            &clean,
            "and is byte-for-byte the input"
        );
    }

    #[test]
    fn overlapping_secrets_leave_no_partial_leak() {
        // One secret is a substring of the other; longest-first ordering must scrub the long one
        // whole rather than leaving the tail of it behind.
        let mut masker = Masker::new();
        masker.register("secret");
        masker.register("secretvalue999");
        let value = json!("x secretvalue999 y");
        let masked = masker.redact_value(&value);
        let out = masked.get().as_ref().as_str().expect("string").to_string();
        assert_eq!(out, "x [REDACTED] y", "the longer secret is scrubbed whole");
        assert!(
            !out.contains("value999") && !out.contains("secret"),
            "no partial residue of either overlapping secret: {out}"
        );
    }

    #[test]
    fn blob_and_message_string_fields_are_scanned() {
        // Blob (base64) and message wrappers are plain string leaves, so the scan reaches them.
        let secret = "c2VjcmV0LWJsb2I="; // base64-ish, above floor
        let masker = masker_with(secret);
        let value = json!({"blob": secret, "message": format!("saw {secret}")});
        let masked = masker.redact_value(&value);
        let text = serde_json::to_string(masked.get()).expect("serialises");
        assert!(
            !text.contains(secret),
            "blob/message leaves are scrubbed: {text}"
        );
        assert_eq!(
            masked.get().as_ref(),
            &json!({"blob": "[REDACTED]", "message": "saw [REDACTED]"}),
            "both wrapper fields redacted"
        );
    }

    #[test]
    fn stringified_number_secret_is_scanned() {
        let secret = "8675309001"; // >= floor, appears as a JSON number leaf
        let masker = masker_with(secret);
        let value: Value = serde_json::from_str(&format!("{{\"pin\": {secret}}}")).expect("json");
        assert!(
            value["pin"].is_number(),
            "the secret arrives as a number leaf"
        );
        let masked = masker.redact_value(&value);
        let text = serde_json::to_string(masked.get()).expect("serialises");
        assert!(
            !text.contains(secret),
            "a secret re-encoded as a number is still caught: {text}"
        );
    }

    #[test]
    fn redact_line_scrubs_log_text() {
        let secret = "topsecretpw"; // >= floor
        let masker = masker_with(secret);
        let line = format!("login with {secret} now");
        let masked = masker.redact_line(&line);
        assert_eq!(masked.get().as_ref(), "login with [REDACTED] now");

        let clean = masker.redact_line("nothing sensitive here");
        assert!(
            matches!(clean.get(), Cow::Borrowed(_)),
            "a clean line is borrowed unchanged"
        );
    }

    // -----------------------------------------------------------------------------------------
    // O2 — paired boundary assertions fail closed; below-floor non-clobber (also above).
    // -----------------------------------------------------------------------------------------

    #[test]
    fn assert_ready_accepts_a_populated_registry() {
        let masker = masker_with("sk-registered-secret");
        masker.assert_ready(&["sk-registered-secret"]); // must not panic
    }

    #[test]
    #[should_panic(expected = "missing a resolved secret")]
    fn assert_ready_trips_when_a_resolved_secret_is_unregistered() {
        // A non-empty registry (so the aggregate emptiness check passes) that is nonetheless
        // missing *this* resolved secret trips the per-secret path independently.
        let masker = masker_with("sk-some-other-registered-secret");
        masker.assert_ready(&["sk-was-resolved-but-not-registered"]);
    }

    #[test]
    #[should_panic(expected = "empty though secrets were resolved")]
    fn assert_ready_trips_on_empty_registry_with_resolved_secrets() {
        let masker = Masker::new();
        // An empty resolved-secret string does not count; a real one must be registered.
        masker.assert_ready(&["real-secret"]);
    }

    #[test]
    fn assert_routed_accepts_a_payload_from_this_masker() {
        let masker = masker_with("sk-secret-value");
        let value = json!("nothing to redact");
        let masked = masker.redact_value(&value);
        masker.assert_routed(&masked); // must not panic
    }

    #[test]
    #[should_panic(expected = "not routed through this Masker")]
    fn assert_routed_trips_on_a_bypassing_emission() {
        // A payload minted by a *different* Masker models an emission that bypassed this run's
        // Masker (it never registered this run's secrets). The output-port assertion catches it.
        let run_masker = masker_with("sk-this-run-secret");
        let foreign_masker = masker_with("sk-other-secret");
        let value = json!("payload");
        let bypassing = foreign_masker.redact_value(&value);
        run_masker.assert_routed(&bypassing);
    }

    #[test]
    fn maskers_have_distinct_identities() {
        let a = Masker::new();
        let b = Masker::new();
        assert_ne!(a.id(), b.id(), "each masker gets a distinct id");
        assert_ne!(a.id(), 0, "and a non-zero id");
    }

    #[test]
    fn register_is_idempotent_and_partitions_by_floor() {
        let mut masker = Masker::new();
        assert!(masker.is_empty(), "starts empty");
        masker.register("longsecret"); // >= floor
        masker.register("longsecret"); // duplicate: no growth
        masker.register("ab"); // below floor
        masker.register(""); // empty: ignored
        assert_eq!(
            masker.len(),
            2,
            "duplicates and empty values do not grow the registry"
        );
        assert!(masker.contains("longsecret") && masker.contains("ab"));
        assert!(!masker.contains(""), "an empty value is never registered");
    }
}
