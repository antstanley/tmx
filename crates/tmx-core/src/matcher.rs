//! The `MatcherEngine` — the pure, sync value-matching primitive shared by `assert` and `eval`.
//!
//! One function, [`MatcherEngine::evaluate`], decides whether an `actual` JSON value satisfies a
//! [`MatcherName`] against an optional list of matcher arguments, respecting a `not` flag. It is the
//! single primitive behind both `assert` (a pass/fail **gate** — fail if any assertion does not
//! hold) and the `eval` `matcher` scorer (a **score** — `1.0` when it passes, `0.0` otherwise); the
//! two never grow a parallel vocabulary
//! ([`.specs/05-fan-out-and-eval.md` §The MatcherEngine](../../../.specs/05-fan-out-and-eval.md#the-matcherengine)).
//!
//! It is **pure**: a plain sync `fn` that takes no port, awaits nothing, allocates only what a deep
//! comparison needs, and does no I/O. Given the same inputs it always returns the same boolean.
//!
//! ## Argument shape
//!
//! The engine takes the matcher's arguments already split into a slice (`expected: Option<&[Value]>`),
//! not the raw schema `expected` field. Splitting is the caller's job (task 11 `assert`, task 19 the
//! scorer): a unary matcher passes `None`; a single-argument matcher passes a one-element slice; a
//! multi-argument matcher such as `toHaveProperty(path, value)` or `toBeCloseTo(number, precision)`
//! passes the arguments in order. Reading `expected[0]` / `expected[1]` here is therefore total —
//! a missing required argument simply makes the matcher **not hold** (its base result is `false`).
//!
//! ## The JSON value model vs Vitest's runtime
//!
//! The vocabulary mirrors Vitest's `expect`, but the subject is a [`serde_json::Value`], not a live
//! JS value, so a few matchers are pinned to what JSON can represent — each documented at its arm:
//!
//! - **`undefined` is not representable.** JSON has no `undefined`, and TMX resolves an absent
//!   reference to a typed error rather than a value, so every value that reaches the engine is
//!   *defined*: `toBeDefined` holds for all of them and `toBeUndefined` for none (matching Vitest,
//!   where `null` is defined). Likewise `NaN` is not a representable `Value`, so `toBeNaN` cannot
//!   hold over a JSON number.
//! - **`toBe` is shallow, `toEqual`/`toStrictEqual` are deep.** `toBe` is `Object.is`: primitives
//!   compare by value (`-0` ≠ `0`), and two compound values are never the same reference, so `toBe`
//!   over an array/object is `false` — use `toEqual` for structural equality (Vitest semantics).
//! - **`toMatch` is substring-only, `toThrow`/`toSatisfy` take a pre-resolved result.** A `RegExp`
//!   and a predicate/thrown-error are not JSON values; the caller pre-resolves them (a substring for
//!   `toMatch`, a boolean for `toThrow`/`toSatisfy`) so the engine stays a pure value comparison.

use serde_json::Value;
use tmx_schema::MatcherName;

// ---------------------------------------------------------------------------------------------
// `toBeCloseTo` tolerance constants.
//
// These are Vitest *algorithm* constants (its `toBeCloseTo` tolerance is `10^-precision / 2`), not
// tunable engine dimensions, so they live here as named locals rather than in `tmx-schema::limits`
// (which is reserved for bounded engine dimensions — state size, depth, fan-out counts). Named all
// the same, so the tolerance formula reads without a bare magic literal.
// ---------------------------------------------------------------------------------------------

/// Vitest's default `numDigits` for `toBeCloseTo` when the precision argument is omitted.
const CLOSE_TO_DEFAULT_PRECISION_DIGITS: i32 = 2;
/// The base of the decimal precision scale in `toBeCloseTo`'s `10^-precision` tolerance.
const CLOSE_TO_PRECISION_BASE: f64 = 10.0;
/// The halving in `toBeCloseTo`'s tolerance `10^-precision / 2` (a value within half a ULP passes).
const CLOSE_TO_TOLERANCE_HALVING: f64 = 2.0;

/// The pure value-matching primitive behind both `assert` and the `eval` `matcher` scorer.
///
/// A stateless zero-sized type: [`MatcherEngine::evaluate`] is an associated function, so there is
/// no engine state to configure and both callers invoke the identical code path (the "shared
/// primitive" contract of [05 §Scorers](../../../.specs/05-fan-out-and-eval.md#scorers)).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatcherEngine;

impl MatcherEngine {
    /// Evaluate `matcher` over `actual` and its `expected` arguments, applying `not`.
    ///
    /// Returns the gate/score boolean: `true` when the assertion holds. `not: true` inverts the
    /// matcher's base result (Vitest's `.not` modifier), so the two callers get one consistent
    /// negation without a separate code path. Sync, I/O-free, and total over every [`MatcherName`].
    #[must_use]
    pub fn evaluate(
        actual: &Value,
        matcher: MatcherName,
        expected: Option<&[Value]>,
        not: bool,
    ) -> bool {
        // `not` inverts the base matcher result — one XOR, not a parallel negated arm per matcher.
        Self::evaluate_base(actual, matcher, expected) ^ not
    }

    /// The un-negated matcher result. Exhaustive over [`MatcherName`] with **no `_` wildcard**, so
    /// adding a variant to the closed Task 02 enum forces a non-exhaustive-match compile error here:
    /// the code and the vocabulary cannot drift.
    fn evaluate_base(actual: &Value, matcher: MatcherName, expected: Option<&[Value]>) -> bool {
        match matcher {
            // --- Equality ---------------------------------------------------------------------
            // `toBe` is `Object.is` (shallow); compound values are never the same reference.
            MatcherName::ToBe => arg(expected, 0).is_some_and(|e| same_value(actual, e)),
            // `toEqual` / `toStrictEqual` are recursive structural equality. On a pure JSON value
            // there is no `undefined`/sparse-array distinction for the two to differ on, so both
            // are one deep comparison.
            MatcherName::ToEqual | MatcherName::ToStrictEqual => {
                arg(expected, 0).is_some_and(|e| deep_equal(actual, e))
            }

            // --- Truthiness & nullishness -----------------------------------------------------
            MatcherName::ToBeTruthy => is_truthy(actual),
            MatcherName::ToBeFalsy => !is_truthy(actual),
            MatcherName::ToBeNull => actual.is_null(),
            // No JSON value is `undefined` (see the module docs), so `toBeDefined` holds for every
            // representable value and `toBeUndefined` for none — matching Vitest, where `null` is
            // defined.
            MatcherName::ToBeUndefined => false,
            MatcherName::ToBeDefined => true,
            // `NaN` is not a representable `serde_json` number; the check is honest but can only
            // ever be `false` over a JSON value.
            MatcherName::ToBeNaN => actual.as_f64().is_some_and(f64::is_nan),

            // --- Type checks ------------------------------------------------------------------
            MatcherName::ToBeTypeOf => arg(expected, 0)
                .and_then(Value::as_str)
                .is_some_and(|t| js_typeof(actual) == t),
            // No JS runtime/classes exist over JSON, so `instanceof` maps a constructor name to the
            // corresponding JSON kind (`Array` → array, `Object` → object, `String`/`Number`/
            // `Boolean` → the scalar); any other name does not match.
            MatcherName::ToBeInstanceOf => match arg(expected, 0).and_then(Value::as_str) {
                Some("Array") => actual.is_array(),
                Some("Object") => actual.is_object(),
                Some("String") => actual.is_string(),
                Some("Number") => actual.is_number(),
                Some("Boolean") => actual.is_boolean(),
                _ => false,
            },

            // --- Numeric ordering -------------------------------------------------------------
            MatcherName::ToBeGreaterThan => num_cmp(actual, expected, |a, b| a > b),
            MatcherName::ToBeGreaterThanOrEqual => num_cmp(actual, expected, |a, b| a >= b),
            MatcherName::ToBeLessThan => num_cmp(actual, expected, |a, b| a < b),
            MatcherName::ToBeLessThanOrEqual => num_cmp(actual, expected, |a, b| a <= b),
            MatcherName::ToBeCloseTo => close_to(actual, expected),

            // --- Containment & length ---------------------------------------------------------
            MatcherName::ToContain => match actual {
                // A string contains a substring; an array contains an element by `===` (shallow).
                Value::String(s) => arg(expected, 0)
                    .and_then(Value::as_str)
                    .is_some_and(|needle| s.contains(needle)),
                Value::Array(items) => arg(expected, 0)
                    .is_some_and(|needle| items.iter().any(|e| strict_eq(e, needle))),
                _ => false,
            },
            MatcherName::ToContainEqual => match actual {
                Value::Array(items) => arg(expected, 0)
                    .is_some_and(|needle| items.iter().any(|e| deep_equal(e, needle))),
                _ => false,
            },
            MatcherName::ToHaveLength => match length_of(actual) {
                Some(len) => arg(expected, 0)
                    .and_then(Value::as_f64)
                    .is_some_and(|want| (len as f64) == want),
                None => false,
            },

            // --- Structural / property --------------------------------------------------------
            MatcherName::ToHaveProperty => has_property(actual, expected),
            MatcherName::ToMatch => match (actual, arg(expected, 0)) {
                // `RegExp` is not a JSON value, so `toMatch` is substring containment on a string.
                (Value::String(s), Some(Value::String(pat))) => s.contains(pat),
                _ => false,
            },
            MatcherName::ToMatchObject => {
                arg(expected, 0).is_some_and(|e| matches_object(actual, e))
            }
            MatcherName::ToBeOneOf => match arg(expected, 0) {
                Some(Value::Array(candidates)) => candidates.iter().any(|c| deep_equal(actual, c)),
                _ => false,
            },

            // --- Predicate / throw (pre-resolved by the caller) -------------------------------
            // A thrown error and a predicate function are not JSON values; the caller pre-resolves
            // them to a boolean, so the engine just reads its truthiness.
            MatcherName::ToThrow | MatcherName::ToSatisfy => {
                arg(expected, 0).is_some_and(is_truthy)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Argument access.
// ---------------------------------------------------------------------------------------------

/// The `i`-th matcher argument, if present. Total: a missing argument yields `None`, which every
/// arm treats as "the matcher does not hold".
fn arg(expected: Option<&[Value]>, i: usize) -> Option<&Value> {
    expected.and_then(|args| args.get(i))
}

// ---------------------------------------------------------------------------------------------
// Truthiness & typeof.
// ---------------------------------------------------------------------------------------------

/// JavaScript truthiness of a JSON value: `false`, `0`/`-0`/`NaN`, `""`, and `null` are falsy;
/// every array and object (even empty) is truthy.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        // A JSON number is falsy only at zero (NaN is not representable); a non-finite or
        // unrepresentable magnitude is treated as truthy.
        Value::Number(_) => v.as_f64().is_none_or(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// The JavaScript `typeof` of a JSON value. `null`, arrays, and objects are all `"object"`.
fn js_typeof(v: &Value) -> &'static str {
    match v {
        Value::Null => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) | Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------------------------------------
// Equality (shallow `Object.is`, strict `===`, deep structural).
// ---------------------------------------------------------------------------------------------

/// `Object.is` equality (`toBe`): primitives by value with `-0` ≠ `+0`; two compound values are
/// distinct references and so never equal (use [`deep_equal`] for structure).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(xf), Some(yf)) => {
                if xf == 0.0 && yf == 0.0 {
                    // `Object.is(-0, 0)` is false — distinguish the sign of zero.
                    xf.is_sign_negative() == yf.is_sign_negative()
                } else {
                    xf == yf
                }
            }
            // Fall back to the raw number equality when a magnitude is not an `f64`.
            _ => x == y,
        },
        _ => false,
    }
}

/// JavaScript `===` (strict) equality: like [`same_value`] but `-0 === 0`. Compound values compare
/// by reference, so they are never `===` here.
fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(xf), Some(yf)) => xf == yf,
            _ => x == y,
        },
        _ => false,
    }
}

/// Recursive structural equality (`toEqual`): numbers compare by value (so `1` equals `1.0`),
/// arrays elementwise, objects by identical key sets and equal values.
///
/// Recursion is bounded by the input JSON's own depth, which the loader caps at
/// [`JSON_DEPTH_MAX`](tmx_schema::limits::JSON_DEPTH_MAX); the engine adds no separate bound.
fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(xf), Some(yf)) => xf == yf,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| deep_equal(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Numeric matchers.
// ---------------------------------------------------------------------------------------------

/// Compare `actual` and the first argument as numbers with `op`; a non-number on either side (or a
/// missing argument) does not hold.
fn num_cmp(actual: &Value, expected: Option<&[Value]>, op: impl Fn(f64, f64) -> bool) -> bool {
    match (actual.as_f64(), arg(expected, 0).and_then(Value::as_f64)) {
        (Some(a), Some(b)) => op(a, b),
        _ => false,
    }
}

/// `toBeCloseTo(number, precision?)`: passes when `|actual - expected| < 10^-precision / 2`
/// (Vitest's tolerance), with `precision` defaulting to
/// [`CLOSE_TO_DEFAULT_PRECISION_DIGITS`]. Two infinities of the same sign are close.
fn close_to(actual: &Value, expected: Option<&[Value]>) -> bool {
    let (a, b) = match (actual.as_f64(), arg(expected, 0).and_then(Value::as_f64)) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    if a.is_infinite() || b.is_infinite() {
        return a == b;
    }
    let digits = arg(expected, 1)
        .and_then(Value::as_i64)
        .map_or(CLOSE_TO_DEFAULT_PRECISION_DIGITS, |d| d as i32);
    let tolerance = CLOSE_TO_PRECISION_BASE.powi(-digits) / CLOSE_TO_TOLERANCE_HALVING;
    (a - b).abs() < tolerance
}

// ---------------------------------------------------------------------------------------------
// Containment, length, structure.
// ---------------------------------------------------------------------------------------------

/// The `length` a value would report in JS: an array's element count or a string's UTF-16 code-unit
/// count. Objects and scalars have no `length`.
fn length_of(v: &Value) -> Option<usize> {
    match v {
        Value::Array(items) => Some(items.len()),
        Value::String(s) => Some(s.encode_utf16().count()),
        _ => None,
    }
}

/// `toHaveProperty(path, value?)`: whether `actual` has a property at `path`, and — when a second
/// argument is given — whether that property deep-equals it.
fn has_property(actual: &Value, expected: Option<&[Value]>) -> bool {
    let Some(path) = arg(expected, 0) else {
        return false;
    };
    let Some(found) = resolve_path(actual, path) else {
        return false;
    };
    match arg(expected, 1) {
        Some(want) => deep_equal(found, want),
        None => true,
    }
}

/// Resolve a property path (a dotted/bracketed string like `a.b[0].c`, or an array of key/index
/// segments) against `root`, returning the value at the path if it exists.
fn resolve_path<'a>(root: &'a Value, path: &Value) -> Option<&'a Value> {
    let segments: Vec<String> = match path {
        Value::String(s) => parse_string_path(s),
        Value::Array(parts) => {
            let mut segs = Vec::with_capacity(parts.len());
            for part in parts {
                segs.push(match part {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    // A non-string/number segment cannot address a property.
                    _ => return None,
                });
            }
            segs
        }
        _ => return None,
    };
    let mut current = root;
    for segment in &segments {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => {
                let index: usize = segment.parse().ok()?;
                items.get(index)?
            }
            _ => return None,
        };
    }
    Some(current)
}

/// Split a Vitest key path (`"a.b[0].c"`, `"a['x'].y"`) into its segments. Dots separate keys;
/// bracketed sections index arrays or address a (optionally quoted) key.
fn parse_string_path(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(std::mem::take(&mut current));
                }
                let mut inner = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    inner.push(d);
                }
                let inner = inner.trim_matches(|q| q == '\'' || q == '"');
                segments.push(inner.to_string());
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// `toMatchObject`: whether `actual` recursively contains the `expected` subset — objects match on
/// the keys `expected` names (extra keys in `actual` are fine), arrays match elementwise at equal
/// length, and scalars must deep-equal.
fn matches_object(actual: &Value, expected: &Value) -> bool {
    match expected {
        Value::Object(want) => match actual {
            Value::Object(have) => want
                .iter()
                .all(|(k, wv)| have.get(k).is_some_and(|hv| matches_object(hv, wv))),
            _ => false,
        },
        Value::Array(want) => match actual {
            Value::Array(have) => {
                want.len() == have.len()
                    && want
                        .iter()
                        .zip(have.iter())
                        .all(|(wv, hv)| matches_object(hv, wv))
            }
            _ => false,
        },
        _ => deep_equal(actual, expected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Evaluate with an explicit argument list (never negated) — the common positive-case helper.
    fn ev(actual: Value, matcher: MatcherName, expected: &[Value]) -> bool {
        MatcherEngine::evaluate(&actual, matcher, Some(expected), false)
    }

    /// Evaluate a unary matcher (no arguments), never negated.
    fn ev0(actual: Value, matcher: MatcherName) -> bool {
        MatcherEngine::evaluate(&actual, matcher, None, false)
    }

    // -----------------------------------------------------------------------------------------
    // Per-matcher pass/fail (and `not` inversion is checked uniformly by the property test below).
    // -----------------------------------------------------------------------------------------

    #[test]
    fn to_be_is_shallow_object_is() {
        // Primitives compare by value …
        assert!(ev(json!(1), MatcherName::ToBe, &[json!(1)]), "1 toBe 1");
        assert!(
            ev(json!("x"), MatcherName::ToBe, &[json!("x")]),
            "\"x\" toBe \"x\""
        );
        assert!(ev(json!(1), MatcherName::ToBe, &[json!(1.0)]), "1 toBe 1.0");
        // … and fail when unequal.
        assert!(
            !ev(json!(1), MatcherName::ToBe, &[json!(2)]),
            "1 not toBe 2"
        );
        // Object.is distinguishes the sign of zero (the toBe/toEqual subtlety).
        assert!(
            !ev(json!(-0.0), MatcherName::ToBe, &[json!(0.0)]),
            "-0 is not toBe 0 (Object.is)"
        );
        // Compound values are distinct references — never toBe (use toEqual instead).
        assert!(
            !ev(json!({"a": 1}), MatcherName::ToBe, &[json!({"a": 1})]),
            "equal objects are not toBe"
        );
        assert!(
            !ev(json!([1, 2]), MatcherName::ToBe, &[json!([1, 2])]),
            "equal arrays are not toBe"
        );
    }

    #[test]
    fn to_equal_and_strict_equal_are_deep() {
        for matcher in [MatcherName::ToEqual, MatcherName::ToStrictEqual] {
            assert!(
                ev(json!({"a": [1, 2]}), matcher, &[json!({"a": [1, 2]})]),
                "{matcher:?} deep-equals nested structure"
            );
            // Deep equality unifies 1 and 1.0 and considers -0 == 0 (unlike toBe).
            assert!(
                ev(json!(-0.0), matcher, &[json!(0.0)]),
                "{matcher:?} treats -0 and 0 as equal"
            );
            assert!(
                !ev(json!({"a": [1, 2]}), matcher, &[json!({"a": [1, 3]})]),
                "{matcher:?} fails on a differing leaf"
            );
            // A superset/subset of keys is unequal.
            assert!(
                !ev(json!({"a": 1}), matcher, &[json!({"a": 1, "b": 2})]),
                "{matcher:?} fails when key sets differ"
            );
        }
    }

    #[test]
    fn truthiness_and_nullishness() {
        assert!(
            ev0(json!("x"), MatcherName::ToBeTruthy),
            "non-empty is truthy"
        );
        assert!(ev0(json!([]), MatcherName::ToBeTruthy), "[] is truthy");
        assert!(!ev0(json!(0), MatcherName::ToBeTruthy), "0 is not truthy");
        assert!(ev0(json!(0), MatcherName::ToBeFalsy), "0 is falsy");
        assert!(
            ev0(json!(""), MatcherName::ToBeFalsy),
            "empty string is falsy"
        );
        assert!(
            !ev0(json!("x"), MatcherName::ToBeFalsy),
            "\"x\" is not falsy"
        );
        assert!(ev0(json!(null), MatcherName::ToBeNull), "null is null");
        assert!(!ev0(json!(0), MatcherName::ToBeNull), "0 is not null");
    }

    #[test]
    fn defined_undefined_and_nan_follow_the_json_value_model() {
        // Every representable value is defined; none is undefined (null included, per Vitest).
        for v in [json!(null), json!(0), json!(""), json!([]), json!({})] {
            assert!(ev0(v.clone(), MatcherName::ToBeDefined), "{v} is defined");
            assert!(
                !ev0(v.clone(), MatcherName::ToBeUndefined),
                "{v} is not undefined"
            );
            // A JSON number is never NaN.
            assert!(!ev0(v, MatcherName::ToBeNaN), "no JSON value is NaN");
        }
    }

    #[test]
    fn type_of_and_instance_of() {
        assert!(ev(json!(1), MatcherName::ToBeTypeOf, &[json!("number")]));
        assert!(ev(json!("x"), MatcherName::ToBeTypeOf, &[json!("string")]));
        assert!(ev(
            json!(true),
            MatcherName::ToBeTypeOf,
            &[json!("boolean")]
        ));
        assert!(
            ev(json!(null), MatcherName::ToBeTypeOf, &[json!("object")]),
            "typeof null is object"
        );
        assert!(
            ev(json!([1]), MatcherName::ToBeTypeOf, &[json!("object")]),
            "typeof array is object"
        );
        assert!(
            !ev(json!(1), MatcherName::ToBeTypeOf, &[json!("string")]),
            "1 is not typeof string"
        );

        assert!(ev(
            json!([1]),
            MatcherName::ToBeInstanceOf,
            &[json!("Array")]
        ));
        assert!(ev(
            json!({"a": 1}),
            MatcherName::ToBeInstanceOf,
            &[json!("Object")]
        ));
        assert!(ev(
            json!("x"),
            MatcherName::ToBeInstanceOf,
            &[json!("String")]
        ));
        assert!(
            !ev(
                json!({"a": 1}),
                MatcherName::ToBeInstanceOf,
                &[json!("Array")]
            ),
            "an object is not an Array"
        );
        assert!(
            !ev(json!(1), MatcherName::ToBeInstanceOf, &[json!("Nonsense")]),
            "an unknown constructor never matches"
        );
    }

    #[test]
    fn numeric_ordering() {
        assert!(ev(json!(3), MatcherName::ToBeGreaterThan, &[json!(2)]));
        assert!(!ev(json!(2), MatcherName::ToBeGreaterThan, &[json!(2)]));
        assert!(ev(
            json!(2),
            MatcherName::ToBeGreaterThanOrEqual,
            &[json!(2)]
        ));
        assert!(!ev(
            json!(1),
            MatcherName::ToBeGreaterThanOrEqual,
            &[json!(2)]
        ));
        assert!(ev(json!(1), MatcherName::ToBeLessThan, &[json!(2)]));
        assert!(!ev(json!(2), MatcherName::ToBeLessThan, &[json!(2)]));
        assert!(ev(json!(2), MatcherName::ToBeLessThanOrEqual, &[json!(2)]));
        assert!(!ev(json!(3), MatcherName::ToBeLessThanOrEqual, &[json!(2)]));
        // A non-number never orders.
        assert!(
            !ev(json!("2"), MatcherName::ToBeGreaterThan, &[json!(1)]),
            "a string does not order numerically"
        );
    }

    #[test]
    fn close_to_uses_precision() {
        // |0.30001 - 0.3| = 1e-5. Default precision 2 (tolerance 5e-3) → close …
        assert!(ev(json!(0.30001), MatcherName::ToBeCloseTo, &[json!(0.3)]));
        // … but explicit precision 5 (tolerance 5e-6) makes the same comparison fail.
        assert!(!ev(
            json!(0.30001),
            MatcherName::ToBeCloseTo,
            &[json!(0.3), json!(5)]
        ));
        // Floating-point round-off still compares close at the default precision.
        assert!(ev(
            json!(0.2 + 0.1),
            MatcherName::ToBeCloseTo,
            &[json!(0.3)]
        ));
        // Far apart never close.
        assert!(!ev(json!(1.0), MatcherName::ToBeCloseTo, &[json!(2.0)]));
        // Precision 0: within 0.5.
        assert!(ev(
            json!(1.4),
            MatcherName::ToBeCloseTo,
            &[json!(1.0), json!(0)]
        ));
    }

    #[test]
    fn contain_and_contain_equal() {
        assert!(ev(json!("hello"), MatcherName::ToContain, &[json!("ell")]));
        assert!(!ev(json!("hello"), MatcherName::ToContain, &[json!("xyz")]));
        assert!(ev(json!([1, 2, 3]), MatcherName::ToContain, &[json!(2)]));
        assert!(!ev(json!([1, 2, 3]), MatcherName::ToContain, &[json!(9)]));
        // toContain is shallow: an equal object is not `===` an array element.
        assert!(
            !ev(
                json!([{"a": 1}]),
                MatcherName::ToContain,
                &[json!({"a": 1})]
            ),
            "toContain is shallow"
        );
        // toContainEqual is deep.
        assert!(
            ev(
                json!([{"a": 1}]),
                MatcherName::ToContainEqual,
                &[json!({"a": 1})]
            ),
            "toContainEqual is deep"
        );
        assert!(!ev(
            json!([{"a": 2}]),
            MatcherName::ToContainEqual,
            &[json!({"a": 1})]
        ));
    }

    #[test]
    fn have_length() {
        assert!(ev(json!([1, 2, 3]), MatcherName::ToHaveLength, &[json!(3)]));
        assert!(!ev(
            json!([1, 2, 3]),
            MatcherName::ToHaveLength,
            &[json!(2)]
        ));
        assert!(ev(json!("abc"), MatcherName::ToHaveLength, &[json!(3)]));
        // An object has no length.
        assert!(
            !ev(json!({"a": 1}), MatcherName::ToHaveLength, &[json!(1)]),
            "an object has no length"
        );
    }

    #[test]
    fn have_property_paths_and_values() {
        let subject = json!({"a": {"b": [10, 20]}, "flag": true});
        // Existence by dotted path.
        assert!(ev(
            subject.clone(),
            MatcherName::ToHaveProperty,
            &[json!("a.b")]
        ));
        // Bracket-indexed path plus a value check (multi-argument matcher).
        assert!(ev(
            subject.clone(),
            MatcherName::ToHaveProperty,
            &[json!("a.b[1]"), json!(20)]
        ));
        // Array-of-segments path form.
        assert!(ev(
            subject.clone(),
            MatcherName::ToHaveProperty,
            &[json!(["a", "b", 0]), json!(10)]
        ));
        // Present path but a mismatched value fails.
        assert!(!ev(
            subject.clone(),
            MatcherName::ToHaveProperty,
            &[json!("flag"), json!(false)]
        ));
        // Absent path fails.
        assert!(!ev(
            subject,
            MatcherName::ToHaveProperty,
            &[json!("a.missing")]
        ));
    }

    #[test]
    fn match_and_match_object() {
        assert!(ev(
            json!("hello world"),
            MatcherName::ToMatch,
            &[json!("o w")]
        ));
        assert!(!ev(json!("hello"), MatcherName::ToMatch, &[json!("xyz")]));
        assert!(
            !ev(json!(123), MatcherName::ToMatch, &[json!("1")]),
            "toMatch needs a string subject"
        );

        let subject = json!({"a": 1, "b": {"c": 2}, "extra": true});
        // A subset of properties matches even with extra keys present.
        assert!(ev(
            subject.clone(),
            MatcherName::ToMatchObject,
            &[json!({"a": 1, "b": {"c": 2}})]
        ));
        assert!(!ev(
            subject,
            MatcherName::ToMatchObject,
            &[json!({"a": 1, "b": {"c": 9}})]
        ));
    }

    #[test]
    fn be_one_of() {
        assert!(ev(json!(2), MatcherName::ToBeOneOf, &[json!([1, 2, 3])]));
        assert!(ev(
            json!({"a": 1}),
            MatcherName::ToBeOneOf,
            &[json!([{"a": 1}, {"a": 2}])]
        ));
        assert!(!ev(json!(9), MatcherName::ToBeOneOf, &[json!([1, 2, 3])]));
        // A non-array candidate list never matches.
        assert!(!ev(json!(1), MatcherName::ToBeOneOf, &[json!(1)]));
    }

    #[test]
    fn throw_and_satisfy_read_a_pre_resolved_result() {
        // The caller pre-resolves the predicate/throw to a boolean; the engine reads its truthiness.
        for matcher in [MatcherName::ToThrow, MatcherName::ToSatisfy] {
            assert!(
                ev(json!(null), matcher, &[json!(true)]),
                "{matcher:?} holds on a truthy pre-resolved result"
            );
            assert!(
                !ev(json!(null), matcher, &[json!(false)]),
                "{matcher:?} fails on a falsy pre-resolved result"
            );
            assert!(
                !ev0(json!(null), matcher),
                "{matcher:?} without an argument does not hold"
            );
        }
    }

    #[test]
    fn not_inverts_a_representative_matcher() {
        // A focused check that `not` wraps the base result rather than being a separate arm.
        assert!(
            !MatcherEngine::evaluate(&json!(1), MatcherName::ToBe, Some(&[json!(1)]), true),
            "not toBe on an equal value fails"
        );
        assert!(
            MatcherEngine::evaluate(&json!(1), MatcherName::ToBe, Some(&[json!(2)]), true),
            "not toBe on an unequal value holds"
        );
    }

    #[test]
    fn a_missing_required_argument_does_not_hold() {
        // Negative space: a matcher that needs an argument, given none, is `false` (and its `not`
        // is `true`) rather than panicking on an out-of-bounds slice access.
        for matcher in [
            MatcherName::ToBe,
            MatcherName::ToEqual,
            MatcherName::ToBeGreaterThan,
            MatcherName::ToContain,
            MatcherName::ToHaveProperty,
            MatcherName::ToMatchObject,
            MatcherName::ToBeOneOf,
        ] {
            assert!(
                !ev0(json!(1), matcher),
                "{matcher:?} without its argument must not hold"
            );
            assert!(
                MatcherEngine::evaluate(&json!(1), matcher, None, true),
                "{matcher:?} without its argument inverts to hold under not"
            );
        }
    }

    // -----------------------------------------------------------------------------------------
    // Exhaustiveness + property tests (hand-rolled, deterministic — matching the crate's style).
    // -----------------------------------------------------------------------------------------

    #[test]
    fn every_matcher_variant_is_dispatched_without_panic() {
        // Runtime companion to the compile-time exhaustive `match`: calling every variant confirms
        // no arm is a stub that panics, and that the closed vocabulary is wholly wired in.
        let actual = json!({"a": 1, "list": [1, 2, 3], "text": "hello"});
        let expected = [json!("a"), json!(1)];
        let mut dispatched = 0_usize;
        for matcher in MatcherName::ALL {
            let base = MatcherEngine::evaluate(&actual, matcher, Some(&expected), false);
            let negated = MatcherEngine::evaluate(&actual, matcher, Some(&expected), true);
            assert_eq!(
                base, !negated,
                "{matcher:?}: not must invert the base result"
            );
            dispatched += 1;
        }
        assert_eq!(
            dispatched,
            MatcherName::COUNT,
            "every matcher in the closed vocabulary was dispatched"
        );
    }

    /// A tiny deterministic LCG so the property loops are reproducible across runs.
    struct Lcg(u64);
    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    /// Generate a bounded random JSON value.
    fn gen_value(rng: &mut Lcg, depth: u32) -> Value {
        if depth == 0 {
            return match rng.below(6) {
                0 => Value::Null,
                1 => json!(rng.below(5) == 0),
                2 => json!(rng.below(7) as i64 - 3),
                3 => json!((rng.below(5) as f64) / 2.0),
                4 => json!(["", "x", "hello"][rng.below(3) as usize]),
                _ => json!(rng.below(3)),
            };
        }
        match rng.below(4) {
            0 => {
                let len = rng.below(4) as usize;
                Value::Array((0..len).map(|_| gen_value(rng, depth - 1)).collect())
            }
            1 => {
                let len = rng.below(4) as usize;
                let mut map = serde_json::Map::new();
                for i in 0..len {
                    map.insert(format!("k{i}"), gen_value(rng, depth - 1));
                }
                Value::Object(map)
            }
            _ => gen_value(rng, 0),
        }
    }

    #[test]
    fn property_not_always_inverts_over_random_inputs() {
        // For every matcher and a wide range of random actual/expected shapes, evaluating with
        // `not: true` is exactly the negation of `not: false` — and nothing panics.
        let mut rng = Lcg(0x0808_0808_dead_beef);
        for _ in 0..3000 {
            let actual = gen_value(&mut rng, 3);
            let args: Vec<Value> = (0..rng.below(3)).map(|_| gen_value(&mut rng, 2)).collect();
            let expected = if args.is_empty() {
                None
            } else {
                Some(args.as_slice())
            };
            for matcher in MatcherName::ALL {
                let base = MatcherEngine::evaluate(&actual, matcher, expected, false);
                let negated = MatcherEngine::evaluate(&actual, matcher, expected, true);
                assert_eq!(
                    base, !negated,
                    "{matcher:?}: not must invert for actual={actual} args={args:?}"
                );
            }
        }
    }

    #[test]
    fn property_deep_equality_is_reflexive() {
        // toEqual/toStrictEqual/toBeOneOf(self) hold for any value against itself — the positive
        // side of the structural-equality property.
        let mut rng = Lcg(0x00c0_ffee_0000_0001);
        for _ in 0..2000 {
            let v = gen_value(&mut rng, 3);
            assert!(
                ev(v.clone(), MatcherName::ToEqual, std::slice::from_ref(&v)),
                "toEqual is reflexive for {v}"
            );
            assert!(
                ev(
                    v.clone(),
                    MatcherName::ToStrictEqual,
                    std::slice::from_ref(&v)
                ),
                "toStrictEqual is reflexive for {v}"
            );
            assert!(
                ev(v.clone(), MatcherName::ToBeOneOf, &[json!([v.clone()])]),
                "a value is one of a set containing it: {v}"
            );
            // toMatchObject against itself also holds (a value contains itself as a subset).
            assert!(
                ev(
                    v.clone(),
                    MatcherName::ToMatchObject,
                    std::slice::from_ref(&v)
                ),
                "toMatchObject is reflexive for {v}"
            );
        }
    }
}
