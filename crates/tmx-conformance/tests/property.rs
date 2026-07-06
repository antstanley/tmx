//! Property-test tier (Task 32 residue; development-guidelines §Testing).
//!
//! `proptest` cases over the three pure services the guidelines name — the interpolation parser, the
//! matcher engine, and state merge. Each states a *universal* invariant and lets proptest search a
//! wide input space for a counterexample; the properties are genuinely total, so the tier is
//! non-flaky. Failure persistence is disabled per-suite, so the tier writes no regression files and
//! stays reproducible under `nextest`.

use proptest::prelude::*;
use serde_json::Value;

use tmx_conformance::empty_scope;
use tmx_core::{MatcherEngine, StateBuilder, evaluate};
use tmx_schema::MatcherName;

/// A bounded JSON value: scalar leaves plus small arrays/objects, at most a handful of levels deep —
/// well under `JSON_DEPTH_MAX`, so a generated value never trips the merge depth guard.
fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| Value::Number(n.into())),
        "[a-z]{0,6}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec(("[a-z]{1,4}", inner), 0..4)
                .prop_map(|pairs| { Value::Object(pairs.into_iter().collect()) }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, cases: 512, ..ProptestConfig::default() })]

    /// The interpolation parser is **total over arbitrary input**: for any string drawn from an
    /// operator/namespace-rich alphabet, `evaluate` returns `Ok` or a *typed resolution* `Err` with a
    /// non-empty code — it never panics and never overflows the stack.
    #[test]
    fn interpolation_parser_never_panics_and_errors_are_typed(
        src in "[a-z.()\\[\\]!=&|\"'0-9 ]{0,48}"
    ) {
        let empty = Value::Object(serde_json::Map::new());
        let scope = empty_scope(&empty);
        if let Err(error) = evaluate(&src, &scope) {
            prop_assert_eq!(
                error.category,
                tmx_core::ErrorCategory::Resolution,
                "a parse/eval failure is always a resolution error"
            );
            prop_assert!(!error.code.is_empty(), "every error carries a stable code");
        }
    }

    /// The matcher engine's `not` modifier is a **uniform total negation**: for every matcher and every
    /// pair of JSON operands, `evaluate(.., not = true)` is exactly the negation of
    /// `evaluate(.., not = false)` — one XOR, no per-matcher special case, and no panic over any input.
    #[test]
    fn matcher_not_is_the_uniform_negation_of_every_matcher(
        actual in json_value(),
        expected in json_value(),
    ) {
        for matcher in MatcherName::ALL {
            let args = [expected.clone()];
            let base = MatcherEngine::evaluate(&actual, matcher, Some(&args), false);
            let negated = MatcherEngine::evaluate(&actual, matcher, Some(&args), true);
            prop_assert_eq!(
                negated,
                !base,
                "`not` inverts {:?} uniformly",
                matcher
            );
        }
    }

    /// The incremental state size is **exact**: after any sequence of merges under the default cap, the
    /// running `size_bytes()` equals a wholesale canonical re-serialisation of the state, and the last
    /// value merged under a key reads back equal (the merge writes what it was given).
    #[test]
    fn state_merge_size_is_exact_and_the_last_write_round_trips(
        merges in prop::collection::vec(("[a-z]{1,5}", json_value()), 1..8)
    ) {
        let mut builder = StateBuilder::new();
        let mut last: Option<(String, Value)> = None;
        for (key, value) in merges {
            // Bounded values under the 512-MiB default cap never over-cap or over-deepen, so every
            // merge succeeds; a rejection here would be a real bug, so it fails the property.
            builder
                .merge(&key, value.clone(), &key)
                .map_err(|error| TestCaseError::fail(format!("merge rejected a bounded value: {error}")))?;
            prop_assert_eq!(
                builder.size_bytes(),
                serde_json::to_string(builder.as_value()).map(|s| s.len() as u64).unwrap_or(0),
                "incremental size equals a wholesale canonical re-serialisation"
            );
            last = Some((key, value));
        }
        if let Some((key, value)) = last {
            prop_assert_eq!(
                builder.as_value().get(&key),
                Some(&value),
                "the last write under a key is readable back, byte-for-byte"
            );
        }
    }
}
