//! State merge, output normalisation, and the state size cap.
//!
//! The single **pure** step the runner takes after each task: normalise the adapter's result into
//! JSON, then merge it into the [`PipelineState`] under the task's resolved key
//! (`output ?? name`), keeping the whole serialised state under
//! [`STATE_SIZE_MAX_BYTES`](tmx_schema::limits::STATE_SIZE_MAX_BYTES) and no deeper than
//! [`JSON_DEPTH_MAX`](tmx_schema::limits::JSON_DEPTH_MAX)
//! ([`.specs/04-execution-engine.md` §State size cap, §Pipeline execution algorithm](../../../.specs/04-execution-engine.md)).
//!
//! Two invariants distinguish a **programmer** bug from **input-reachable** state:
//!
//! - The state is always a JSON object and the merge key is non-empty — `assert!`ed, because only a
//!   caller bug can break them.
//! - The state stays under the size cap and the depth bound — returned as a **typed** [`RunError`]
//!   (`state_cap_exceeded` / `json_too_deep`), *and* asserted as a backstop, because a large or deep
//!   task output can trip them from ordinary input. The typed error is constructed first, so a real
//!   workload gets a clean abort naming the offending task rather than a panic.
//!
//! Size is the **canonical-JSON byte length** (UTF-8, no insignificant whitespace), tracked
//! **incrementally** by [`StateBuilder`]: each merge adjusts a running byte count by exactly the
//! delta the mutation adds or removes, which equals a wholesale `serde_json` re-serialisation of the
//! resulting state (the total is order-independent, so `serde_json`'s sorted-key rendering and the
//! incremental accounting always agree). Re-serialising the whole state on every merge would be
//! O(state) per task — the incremental count keeps it O(output).

use serde_json::Value;
use tmx_schema::limits::{JSON_DEPTH_MAX, STATE_SIZE_MAX_BYTES};

use crate::error::RunError;
use crate::model::{BlobWrapper, MessageWrapper, PipelineState};

/// The raw result an executor adapter hands back, before normalisation into the Pipeline state.
///
/// An adapter returns one of three shapes; [`normalize_output`] turns each into a JSON [`Value`] so
/// the state stays JSON objects all the way down (01 §Runtime entities):
///
/// - [`Json`](AdapterOutput::Json) — a structured result, used as-is.
/// - [`Text`](AdapterOutput::Text) — UTF-8 text the adapter already knows is text, wrapped as
///   `{ "message": … }`.
/// - [`Bytes`](AdapterOutput::Bytes) — raw bytes, wrapped as `{ "message": … }` when they are valid
///   UTF-8 and `{ "blob": <base64> }` otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterOutput {
    /// A structured JSON result, merged unchanged.
    Json(Value),
    /// UTF-8 text, normalised to `{ "message": … }`.
    Text(String),
    /// Raw bytes: `{ "message": … }` if valid UTF-8, else `{ "blob": <base64> }`.
    Bytes(Vec<u8>),
}

/// Normalise an [`AdapterOutput`] into the JSON [`Value`] that gets merged into the state.
///
/// A non-JSON result is wrapped so `PipelineState` stays JSON objects all the way down: valid UTF-8
/// text becomes `{ "message": … }`, non-UTF-8 bytes become `{ "blob": <base64> }` (the base64
/// payload itself counts toward the state cap). A [`Json`](AdapterOutput::Json) result passes through
/// unchanged.
#[must_use]
pub fn normalize_output(output: AdapterOutput) -> Value {
    match output {
        AdapterOutput::Json(value) => value,
        AdapterOutput::Text(text) => wrap_message(text),
        AdapterOutput::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => wrap_message(text),
            Err(err) => wrap_blob(&err.into_bytes()),
        },
    }
}

/// Wrap UTF-8 text as the `{ "message": … }` normal form. Total: serialising a `MessageWrapper`
/// (a fixed two-field struct) cannot fail, so an impossible error collapses to `Value::Null`, which
/// the caller-side depth/cap checks still handle rather than panicking.
fn wrap_message(text: String) -> Value {
    serde_json::to_value(MessageWrapper { message: text }).unwrap_or(Value::Null)
}

/// Wrap bytes as the `{ "blob": <base64> }` normal form.
fn wrap_blob(bytes: &[u8]) -> Value {
    serde_json::to_value(BlobWrapper {
        blob: base64_encode(bytes),
    })
    .unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------------------------
// Base64 — a tiny, self-contained standard (RFC 4648) encoder.
//
// The pure core takes on no `base64` crate dependency for one normalisation path; standard base64
// with `=` padding is a fixed alphabet and a fixed 3-byte→4-char transform, so it is a handful of
// bounded lines rather than an inward dependency edge. It is exercised against the RFC 4648 test
// vectors in the tests below.
// ---------------------------------------------------------------------------------------------

/// The 64-character standard base64 alphabet (RFC 4648 §4): `A–Z`, `a–z`, `0–9`, `+`, `/`.
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// The pad character appended so the output length is always a multiple of four.
const BASE64_PAD: u8 = b'=';
/// Input bytes consumed per output quantum.
const BASE64_BYTES_PER_GROUP: usize = 3;

/// Encode `bytes` as standard base64 (RFC 4648, `=`-padded). Pure and allocation-bounded: the output
/// is exactly `4 · ceil(len / 3)` ASCII characters.
#[must_use]
fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(BASE64_BYTES_PER_GROUP) * 4);
    for group in bytes.chunks(BASE64_BYTES_PER_GROUP) {
        // Pack up to three bytes into a 24-bit big-endian buffer, missing bytes read as zero.
        let b0 = u32::from(group[0]);
        let b1 = group.get(1).map_or(0, |&b| u32::from(b));
        let b2 = group.get(2).map_or(0, |&b| u32::from(b));
        let packed = (b0 << 16) | (b1 << 8) | b2;
        // Four 6-bit indices, most-significant first.
        let idx = [
            (packed >> 18) & 0x3f,
            (packed >> 12) & 0x3f,
            (packed >> 6) & 0x3f,
            packed & 0x3f,
        ];
        // The last group emits pad characters for the bytes it did not have.
        let emit = group.len() + 1;
        for (i, &six) in idx.iter().enumerate() {
            let ch = if i < emit {
                BASE64_ALPHABET[six as usize]
            } else {
                BASE64_PAD
            };
            out.push(char::from(ch));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// StateBuilder — the incremental-size merge target.
// ---------------------------------------------------------------------------------------------

/// A [`PipelineState`] plus its running canonical-JSON byte size, the target the runner merges each
/// task's output into.
///
/// The size is maintained **incrementally**: [`merge`](StateBuilder::merge) adjusts the byte count by
/// the exact delta of the mutation, so it never re-serialises the whole state, yet always equals a
/// wholesale `serde_json` re-serialisation of the current state.
///
/// The cap defaults to [`STATE_SIZE_MAX_BYTES`]; [`with_cap`](StateBuilder::with_cap) sets a lower
/// one (the seam the `--max-state-size` flag / `limits.maxStateSize` key / `TMX_MAX_STATE_SIZE`
/// env raise, per 04 §State size cap). The cap can only be *narrowed* below the hard ceiling, never
/// widened past it, so the engine's memory bound holds regardless of configuration.
#[derive(Debug, Clone)]
pub struct StateBuilder {
    state: Value,
    size_bytes: u64,
    cap_bytes: u64,
}

impl StateBuilder {
    /// Start from an empty state (`{}`), whose canonical form is two bytes, capped at the default
    /// [`STATE_SIZE_MAX_BYTES`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_cap(STATE_SIZE_MAX_BYTES)
    }

    /// Start from an empty state with an explicit cap, clamped to at most the hard
    /// [`STATE_SIZE_MAX_BYTES`] ceiling — a configured cap can lower the bound but never raise it.
    #[must_use]
    pub fn with_cap(cap_bytes: u64) -> Self {
        let state = PipelineState::empty().into_value();
        let size_bytes = canonical_len(&state);
        Self {
            state,
            size_bytes,
            cap_bytes: cap_bytes.min(STATE_SIZE_MAX_BYTES),
        }
    }

    /// Start from an existing (seeded) [`PipelineState`], computing its canonical size once up front,
    /// capped at the default [`STATE_SIZE_MAX_BYTES`].
    #[must_use]
    pub fn from_state(state: PipelineState) -> Self {
        let state = state.into_value();
        let size_bytes = canonical_len(&state);
        Self {
            state,
            size_bytes,
            cap_bytes: STATE_SIZE_MAX_BYTES,
        }
    }

    /// The effective size cap in canonical-JSON bytes (at most [`STATE_SIZE_MAX_BYTES`]).
    #[must_use]
    pub fn cap_bytes(&self) -> u64 {
        self.cap_bytes
    }

    /// The current canonical-JSON byte size of the state — the quantity bounded by
    /// [`STATE_SIZE_MAX_BYTES`].
    #[must_use]
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// The current state as a borrowed [`PipelineState`]-shaped [`Value`] (always an object).
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.state
    }

    /// Consume the builder, yielding the merged [`PipelineState`]. Total: the object invariant is
    /// held on entry and preserved by every merge, so reconstruction cannot fail; an impossible
    /// failure collapses to an empty state rather than panicking.
    #[must_use]
    pub fn into_state(self) -> PipelineState {
        PipelineState::new(self.state).unwrap_or_else(|_| PipelineState::empty())
    }

    /// Merge `output` into the state under `key` (the task's resolved `output ?? name`), naming
    /// `task` in any cap error.
    ///
    /// Writes `state[key] = output`, overwriting a prior value under the same key. Enforces two
    /// input-reachable bounds as typed errors *and* asserted backstops:
    ///
    /// - **Depth.** A merged document deeper than [`JSON_DEPTH_MAX`] is a
    ///   [`RunError::validation`] (`json_too_deep`) — rejected before the state is touched.
    /// - **Size.** A merge that would push the canonical state size over [`STATE_SIZE_MAX_BYTES`]
    ///   is a [`RunError::run_failure`] (`state_cap_exceeded`) naming `task` — rejected before the
    ///   state is touched, so the state never transiently exceeds the cap.
    ///
    /// # Panics
    ///
    /// Asserts (programmer-bug backstops) that `key` is non-empty and that the state is a JSON
    /// object on entry.
    pub fn merge(&mut self, key: &str, output: Value, task: &str) -> Result<(), RunError> {
        assert!(
            !key.is_empty(),
            "the merge key (output ?? name) must be non-empty"
        );
        assert!(
            self.state.is_object(),
            "the Pipeline state must be a JSON object before a merge"
        );

        // Depth: reject an over-deep document before mutating, so a rejected merge leaves the state
        // (and its size) untouched.
        check_merge_depth(&output, key, task)?;

        // Incremental size: compute the post-merge byte size from the current size and the delta of
        // this one key, without re-serialising the whole state.
        let value_len = canonical_len(&output);
        let key_token_len = key_token_len(key);

        // Borrow the state object directly. The `is_object` assertion above guarantees the `Object`
        // arm; the `else` is unreachable, but returning a typed error keeps a release build off a
        // panicking `expect`/`unwrap` path (denied workspace-wide) even if the invariant were broken.
        let Value::Object(map) = &mut self.state else {
            return Err(RunError::run_failure(
                "state_not_object",
                "the Pipeline state must be a JSON object before a merge",
            )
            .with_task(task));
        };
        let new_size = match map.get(key) {
            // Overwrite: swap the old value's serialisation for the new one (delta may be negative).
            Some(existing) => {
                let old_len = canonical_len(existing);
                self.size_bytes - old_len + value_len
            }
            // Insert: add the `"key":value` element, plus a comma separator unless it is the first.
            None => {
                let element = key_token_len + 1 + value_len; // `"key"` + `:` + value
                let separator = u64::from(!map.is_empty()); // a leading `,` after the first element
                self.size_bytes + element + separator
            }
        };

        if new_size > self.cap_bytes {
            // Input-reachable: return the typed error naming the task *before* the assertion, so a
            // real over-cap workload gets a clean abort, not a panic.
            let cap = self.cap_bytes;
            let err = RunError::run_failure(
                "state_cap_exceeded",
                format!(
                    "merging task {task:?} would grow the Pipeline state to {new_size} bytes, over the {cap}-byte cap"
                ),
            )
            .with_task(task);
            debug_assert!(
                new_size > self.cap_bytes,
                "the cap check and the reported error must agree"
            );
            return Err(err);
        }

        // Commit: mutate the state and adopt the pre-computed size.
        map.insert(key.to_string(), output);
        self.size_bytes = new_size;

        // Postcondition backstop: the incrementally-tracked size equals a wholesale re-serialisation.
        debug_assert_eq!(
            self.size_bytes,
            canonical_len(&self.state),
            "incremental size must equal a wholesale canonical re-serialisation"
        );
        Ok(())
    }
}

impl Default for StateBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The canonical-JSON byte length of `value` — a compact `serde_json` render (UTF-8, no insignificant
/// whitespace). Total: `serde_json` cannot fail to serialise a `Value`, so an impossible error yields
/// `0`, which only ever *under*-counts and so never hides an over-cap merge behind a false pass.
#[must_use]
fn canonical_len(value: &Value) -> u64 {
    serde_json::to_string(value).map_or(0, |s| s.len() as u64)
}

/// The canonical byte length of `key` rendered as a JSON string token, including quotes and any
/// escaping — the width the key contributes to the object's serialisation.
#[must_use]
fn key_token_len(key: &str) -> u64 {
    serde_json::to_string(key).map_or(0, |s| s.len() as u64)
}

/// Reject a merged document nested deeper than [`JSON_DEPTH_MAX`] with a typed `json_too_deep`
/// [`RunError::validation`].
///
/// The check is **iterative** (an explicit stack, not recursion), so a pathologically deep value can
/// never overflow the stack before it is rejected. `output` sits one level below the top-level state
/// object, so its own nodes start at state-depth 2.
fn check_merge_depth(output: &Value, key: &str, task: &str) -> Result<(), RunError> {
    /// The state-depth of `output`'s own root: the top-level state object is depth 1.
    const OUTPUT_ROOT_DEPTH: u32 = 2;

    let mut stack: Vec<(&Value, u32)> = vec![(output, OUTPUT_ROOT_DEPTH)];
    while let Some((value, depth)) = stack.pop() {
        if depth > JSON_DEPTH_MAX {
            return Err(RunError::validation(
                "json_too_deep",
                format!(
                    "merging task {task:?} under key {key:?} would nest the Pipeline state deeper than the {JSON_DEPTH_MAX}-level cap"
                ),
            )
            .with_task(task));
        }
        match value {
            Value::Array(items) => {
                for item in items {
                    stack.push((item, depth + 1));
                }
            }
            Value::Object(entries) => {
                for entry in entries.values() {
                    stack.push((entry, depth + 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a value that is `arrays` nested single-element arrays wrapping a scalar leaf, so its
    /// own `value_depth` is `arrays + 1`.
    fn nested_arrays(arrays: u32) -> Value {
        let mut value = Value::Number(0.into());
        for _ in 0..arrays {
            value = Value::Array(vec![value]);
        }
        value
    }

    #[test]
    fn normalize_passes_json_through_and_wraps_text_as_message() {
        let json = serde_json::json!({ "ok": true, "n": 3 });
        assert_eq!(
            normalize_output(AdapterOutput::Json(json.clone())),
            json,
            "a Json result is merged unchanged"
        );

        let wrapped = normalize_output(AdapterOutput::Text("hello".to_string()));
        assert_eq!(
            wrapped,
            serde_json::json!({ "message": "hello" }),
            "text becomes a message wrapper"
        );
        // Bytes that are valid UTF-8 are text, so they also become a message wrapper.
        let from_bytes = normalize_output(AdapterOutput::Bytes(b"hi".to_vec()));
        assert_eq!(
            from_bytes,
            serde_json::json!({ "message": "hi" }),
            "valid-UTF-8 bytes normalise to a message, not a blob"
        );
    }

    #[test]
    fn normalize_wraps_non_utf8_bytes_as_base64_blob() {
        // 0xFF 0xFE is not valid UTF-8, so it must go to a base64 blob, never a message.
        let value = normalize_output(AdapterOutput::Bytes(vec![0xFF, 0xFE]));
        assert_eq!(
            value,
            serde_json::json!({ "blob": "//4=" }),
            "non-UTF-8 bytes normalise to a base64 blob"
        );
        assert!(
            value.get("message").is_none(),
            "a binary result never produces a message key"
        );
    }

    #[test]
    fn base64_matches_the_rfc4648_vectors() {
        // The canonical RFC 4648 §10 test vectors pin the encoder exactly, padding included.
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in vectors {
            assert_eq!(
                base64_encode(input.as_bytes()),
                expected,
                "base64({input:?}) must equal the RFC 4648 vector"
            );
            assert_eq!(
                base64_encode(input.as_bytes()).len() % 4,
                0,
                "base64 output is always a multiple of four characters"
            );
        }
    }

    #[test]
    fn merge_writes_under_the_resolved_key_and_overwrites() {
        let mut builder = StateBuilder::new();
        builder
            .merge("greeting", serde_json::json!("hi"), "greet")
            .expect("first merge is under the cap");
        assert_eq!(
            builder.as_value(),
            &serde_json::json!({ "greeting": "hi" }),
            "the output lands under the resolved key"
        );

        // A second key extends the object; the resolved key (output ?? name) need not equal the task
        // name — here the task `probe` writes under key `status`.
        builder
            .merge("status", serde_json::json!(200), "probe")
            .expect("second merge is under the cap");
        assert_eq!(
            builder.as_value(),
            &serde_json::json!({ "greeting": "hi", "status": 200 }),
            "a distinct key is added alongside"
        );

        // Overwriting an existing key replaces the value in place — the size delta is negative here.
        builder
            .merge("greeting", serde_json::json!("hello there"), "greet")
            .expect("overwrite is under the cap");
        assert_eq!(
            builder.as_value(),
            &serde_json::json!({ "greeting": "hello there", "status": 200 }),
            "an existing key is overwritten, not duplicated"
        );
    }

    #[test]
    fn incremental_size_equals_a_wholesale_reserialization() {
        // Drive a representative sequence — inserts, an overwrite that shrinks, nested containers,
        // a blob — and assert the incrementally-tracked size equals a full re-serialisation at every
        // step. This is exactly the O(output)-vs-O(state) equivalence the incremental accounting
        // promises.
        let mut builder = StateBuilder::new();
        assert_eq!(builder.size_bytes(), 2, "empty state is `{{}}` — two bytes");

        let steps: [(&str, Value, &str); 5] = [
            ("a", serde_json::json!("first"), "task_a"),
            (
                "b",
                serde_json::json!({ "nested": [1, 2, 3], "deep": { "x": true } }),
                "task_b",
            ),
            ("a", serde_json::json!(0), "task_a"), // overwrite: a shrinking delta
            (
                "c",
                normalize_output(AdapterOutput::Bytes(vec![0x00, 0x10, 0xFF])),
                "task_c",
            ),
            ("d", serde_json::json!([]), "task_d"),
        ];
        for (key, output, task) in steps {
            builder.merge(key, output, task).expect("under the cap");
            assert_eq!(
                builder.size_bytes(),
                canonical_len(builder.as_value()),
                "incremental size must equal a wholesale re-serialisation after merging {key:?}"
            );
        }
        assert!(
            builder.size_bytes() > 2,
            "a non-trivial state is larger than the empty object"
        );
    }

    #[test]
    fn merge_below_at_and_above_the_cap() {
        // Boundary tests run against a small configured cap (via `with_cap`) rather than the 512-MiB
        // hard ceiling, so at/above-cap payloads stay tiny; `default_cap_is_the_hard_ceiling` below
        // proves the default `new()` wires the real `STATE_SIZE_MAX_BYTES` constant.
        //
        // Empty state is `{}` (2 bytes); inserting the first element `"k":"<payload>"` adds
        // key_token + colon + value_token(2 + payload_len). Pick a cap and derive the payload length
        // that lands the state exactly on it.
        const KEY: &str = "k";
        const CAP: u64 = 256;
        let overhead: u64 = 2 + key_token_len(KEY) + 1 + 2; // {} + "k" + : + the two value quotes
        let at_cap_payload = (CAP - overhead) as usize;

        // One below the cap: a payload one byte shorter succeeds and leaves size == cap - 1.
        let mut below = StateBuilder::with_cap(CAP);
        below
            .merge(KEY, Value::String("x".repeat(at_cap_payload - 1)), "big")
            .expect("a merge one byte below the cap succeeds");
        assert_eq!(
            below.size_bytes(),
            CAP - 1,
            "the below-cap merge lands exactly one byte under the cap"
        );

        // Exactly at the cap: a merge whose result equals the cap is allowed (the bound is `<=`).
        let mut at = StateBuilder::with_cap(CAP);
        at.merge(KEY, Value::String("x".repeat(at_cap_payload)), "big")
            .expect("a merge exactly at the cap succeeds");
        assert_eq!(
            at.size_bytes(),
            CAP,
            "the at-cap merge lands exactly on the cap"
        );

        // One above the cap: negative space — a one-byte-larger value is a typed error, not a panic,
        // and it names the offending task. The state is left untouched.
        let mut over = StateBuilder::with_cap(CAP);
        let err = over
            .merge(
                KEY,
                Value::String("x".repeat(at_cap_payload + 1)),
                "uploader",
            )
            .expect_err("a merge one byte over the cap is rejected");
        assert_eq!(
            err.code, "state_cap_exceeded",
            "the over-cap error carries the stable code"
        );
        assert_eq!(
            err.category,
            crate::error::ErrorCategory::RunFailure,
            "an over-cap merge is a run failure"
        );
        assert_eq!(
            err.task.as_deref(),
            Some("uploader"),
            "the over-cap error names the offending task"
        );
        assert_eq!(
            over.size_bytes(),
            2,
            "a rejected merge leaves the state (and its size) untouched"
        );
    }

    #[test]
    fn default_cap_is_the_hard_ceiling_and_cannot_be_widened() {
        // The default builder caps at exactly the named `STATE_SIZE_MAX_BYTES` constant, and a
        // configured cap can only narrow the bound — never raise it past the hard ceiling.
        assert_eq!(
            StateBuilder::new().cap_bytes(),
            STATE_SIZE_MAX_BYTES,
            "the default cap is the hard ceiling constant"
        );
        assert_eq!(
            StateBuilder::with_cap(STATE_SIZE_MAX_BYTES + 1_000_000).cap_bytes(),
            STATE_SIZE_MAX_BYTES,
            "an over-ceiling configured cap is clamped down to the hard ceiling"
        );
        assert_eq!(
            StateBuilder::with_cap(1024).cap_bytes(),
            1024,
            "a below-ceiling configured cap is honoured"
        );
    }

    #[test]
    fn merge_rejects_a_document_deeper_than_the_depth_cap() {
        // `output` nests one level below the top state object, so a value of its own depth
        // `JSON_DEPTH_MAX - 1` sits at the state depth cap and is accepted; one deeper is rejected.
        let deepest_ok_arrays = JSON_DEPTH_MAX - 2; // arrays + 1 leaf = JSON_DEPTH_MAX - 1 value_depth
        let mut ok = StateBuilder::new();
        ok.merge("d", nested_arrays(deepest_ok_arrays), "nester")
            .expect("a document at the depth cap is accepted");

        let mut too_deep = StateBuilder::new();
        let err = too_deep
            .merge("d", nested_arrays(deepest_ok_arrays + 1), "nester")
            .expect_err("a document one level over the depth cap is rejected");
        assert_eq!(
            err.code, "json_too_deep",
            "the over-deep error carries the stable code"
        );
        assert_eq!(
            err.category,
            crate::error::ErrorCategory::Validation,
            "an over-deep document is a validation error"
        );
        assert_eq!(
            too_deep.size_bytes(),
            2,
            "a depth-rejected merge leaves the state untouched"
        );
    }

    #[test]
    #[should_panic(expected = "non-empty")]
    fn merge_asserts_a_non_empty_key() {
        // Programmer-bug backstop: an empty resolved key is unreachable from valid input (resolution
        // guarantees a non-empty `output ?? name`), so it is an assertion, not a typed error.
        let mut builder = StateBuilder::new();
        let _ = builder.merge("", serde_json::json!(1), "oops");
    }
}
