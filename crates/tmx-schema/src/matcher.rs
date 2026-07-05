//! The `MatcherName` vocabulary — the closed set of Vitest `expect` value matchers.
//!
//! This is the shared value-matching primitive behind both `assert` (a pass/fail gate) and the
//! `eval` `matcher` scorer (scoring `1.0` when it passes, `0.0` otherwise) — one vocabulary, not
//! two (05 §The MatcherEngine). Only **value** matchers are included; mock- and promise-only
//! matchers (`toHaveBeenCalled`, `resolves`, `rejects`, …) are excluded, mirroring the
//! `matcherName` `$def` in [`docs/tmx.schema.json`](../../../docs/tmx.schema.json).
//!
//! The set is **closed** at [`MatcherName::COUNT`] variants: the schema rejects any other spelling
//! at validation, so an unknown matcher cannot reach the engine. Three independent mechanisms keep
//! the enum, its wire spellings, and the schema from drifting apart:
//!
//! - each variant pins its schema spelling with an explicit `#[serde(rename = …)]`, greppable 1:1
//!   against the schema enum;
//! - [`MatcherName::as_str`] is an **exhaustive** match, so adding a variant without a spelling
//!   fails to compile (the "added matcher" negative space);
//! - [`MatcherName::ALL`] enumerates every variant with a fixed length, so dropping one fails to
//!   compile and the round-trip test re-checks the count (the "dropped matcher" negative space).

use serde::{Deserialize, Serialize};

/// A Vitest `expect` value-matcher name — the closed shared matching primitive of `assert` and the
/// `eval` `matcher` scorer.
///
/// Deserialises from and serialises to the exact schema spelling (`toBe`, `toEqual`, …). The enum
/// is closed at [`MatcherName::COUNT`]; see the module docs for the anti-drift mechanisms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MatcherName {
    /// `toBe` — `Object.is` / referential equality.
    #[serde(rename = "toBe")]
    ToBe,
    /// `toEqual` — recursive structural equality.
    #[serde(rename = "toEqual")]
    ToEqual,
    /// `toStrictEqual` — structural equality, type- and sparseness-strict.
    #[serde(rename = "toStrictEqual")]
    ToStrictEqual,
    /// `toBeTruthy` — the value is JS-truthy.
    #[serde(rename = "toBeTruthy")]
    ToBeTruthy,
    /// `toBeFalsy` — the value is JS-falsy.
    #[serde(rename = "toBeFalsy")]
    ToBeFalsy,
    /// `toBeNull` — the value is `null`.
    #[serde(rename = "toBeNull")]
    ToBeNull,
    /// `toBeUndefined` — the value is `undefined`.
    #[serde(rename = "toBeUndefined")]
    ToBeUndefined,
    /// `toBeDefined` — the value is not `undefined`.
    #[serde(rename = "toBeDefined")]
    ToBeDefined,
    /// `toBeNaN` — the value is `NaN`.
    #[serde(rename = "toBeNaN")]
    ToBeNaN,
    /// `toBeTypeOf` — `typeof value` equals the expected type string.
    #[serde(rename = "toBeTypeOf")]
    ToBeTypeOf,
    /// `toBeInstanceOf` — the value is an instance of the expected class.
    #[serde(rename = "toBeInstanceOf")]
    ToBeInstanceOf,
    /// `toBeGreaterThan` — numeric `>`.
    #[serde(rename = "toBeGreaterThan")]
    ToBeGreaterThan,
    /// `toBeGreaterThanOrEqual` — numeric `>=`.
    #[serde(rename = "toBeGreaterThanOrEqual")]
    ToBeGreaterThanOrEqual,
    /// `toBeLessThan` — numeric `<`.
    #[serde(rename = "toBeLessThan")]
    ToBeLessThan,
    /// `toBeLessThanOrEqual` — numeric `<=`.
    #[serde(rename = "toBeLessThanOrEqual")]
    ToBeLessThanOrEqual,
    /// `toBeCloseTo` — numeric equality within a given precision.
    #[serde(rename = "toBeCloseTo")]
    ToBeCloseTo,
    /// `toContain` — the collection/string contains the expected element/substring.
    #[serde(rename = "toContain")]
    ToContain,
    /// `toContainEqual` — the collection contains a structurally-equal element.
    #[serde(rename = "toContainEqual")]
    ToContainEqual,
    /// `toHaveLength` — the value's `length` equals the expected length.
    #[serde(rename = "toHaveLength")]
    ToHaveLength,
    /// `toHaveProperty` — the value has the property at the given path (optionally with a value).
    #[serde(rename = "toHaveProperty")]
    ToHaveProperty,
    /// `toMatch` — the string matches the expected substring or regular expression.
    #[serde(rename = "toMatch")]
    ToMatch,
    /// `toMatchObject` — the object matches the expected subset of properties.
    #[serde(rename = "toMatchObject")]
    ToMatchObject,
    /// `toBeOneOf` — the value equals one of the expected candidates.
    #[serde(rename = "toBeOneOf")]
    ToBeOneOf,
    /// `toThrow` — the subject throws (optionally matching the expected message/type).
    #[serde(rename = "toThrow")]
    ToThrow,
    /// `toSatisfy` — the value satisfies the given predicate.
    #[serde(rename = "toSatisfy")]
    ToSatisfy,
}

impl MatcherName {
    /// The size of the closed value-matcher vocabulary — every variant of [`MatcherName`].
    ///
    /// Pinned to the length of [`MatcherName::ALL`], so it tracks the enum automatically and cannot
    /// silently disagree with it. The schema's `matcherName` enum has the same cardinality.
    pub const COUNT: usize = Self::ALL.len();

    /// Every matcher variant, in schema-declaration order.
    ///
    /// The array's fixed length pins the closed-vocabulary size: dropping a variant makes this
    /// literal too short and fails to compile; adding one is caught instead by the exhaustive match
    /// in [`MatcherName::as_str`]. Downstream code (the `MatcherEngine`, `lint`) iterates this to
    /// enumerate the vocabulary without a third-party derive.
    pub const ALL: [MatcherName; 25] = [
        MatcherName::ToBe,
        MatcherName::ToEqual,
        MatcherName::ToStrictEqual,
        MatcherName::ToBeTruthy,
        MatcherName::ToBeFalsy,
        MatcherName::ToBeNull,
        MatcherName::ToBeUndefined,
        MatcherName::ToBeDefined,
        MatcherName::ToBeNaN,
        MatcherName::ToBeTypeOf,
        MatcherName::ToBeInstanceOf,
        MatcherName::ToBeGreaterThan,
        MatcherName::ToBeGreaterThanOrEqual,
        MatcherName::ToBeLessThan,
        MatcherName::ToBeLessThanOrEqual,
        MatcherName::ToBeCloseTo,
        MatcherName::ToContain,
        MatcherName::ToContainEqual,
        MatcherName::ToHaveLength,
        MatcherName::ToHaveProperty,
        MatcherName::ToMatch,
        MatcherName::ToMatchObject,
        MatcherName::ToBeOneOf,
        MatcherName::ToThrow,
        MatcherName::ToSatisfy,
    ];

    /// The canonical schema spelling of this matcher (`MatcherName::ToBe` → `"toBe"`).
    ///
    /// Exhaustive by construction: a variant added to the enum without a spelling here fails to
    /// compile, so the code and the closed schema set cannot drift. This is the allocation-free,
    /// sync accessor the pure core uses (the serde `Serialize` impl produces the identical string,
    /// which the round-trip test asserts).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            MatcherName::ToBe => "toBe",
            MatcherName::ToEqual => "toEqual",
            MatcherName::ToStrictEqual => "toStrictEqual",
            MatcherName::ToBeTruthy => "toBeTruthy",
            MatcherName::ToBeFalsy => "toBeFalsy",
            MatcherName::ToBeNull => "toBeNull",
            MatcherName::ToBeUndefined => "toBeUndefined",
            MatcherName::ToBeDefined => "toBeDefined",
            MatcherName::ToBeNaN => "toBeNaN",
            MatcherName::ToBeTypeOf => "toBeTypeOf",
            MatcherName::ToBeInstanceOf => "toBeInstanceOf",
            MatcherName::ToBeGreaterThan => "toBeGreaterThan",
            MatcherName::ToBeGreaterThanOrEqual => "toBeGreaterThanOrEqual",
            MatcherName::ToBeLessThan => "toBeLessThan",
            MatcherName::ToBeLessThanOrEqual => "toBeLessThanOrEqual",
            MatcherName::ToBeCloseTo => "toBeCloseTo",
            MatcherName::ToContain => "toContain",
            MatcherName::ToContainEqual => "toContainEqual",
            MatcherName::ToHaveLength => "toHaveLength",
            MatcherName::ToHaveProperty => "toHaveProperty",
            MatcherName::ToMatch => "toMatch",
            MatcherName::ToMatchObject => "toMatchObject",
            MatcherName::ToBeOneOf => "toBeOneOf",
            MatcherName::ToThrow => "toThrow",
            MatcherName::ToSatisfy => "toSatisfy",
        }
    }
}

// The closed vocabulary is exactly 25 value matchers (05 §The MatcherEngine; the `matcherName`
// `$def`). Pinning the literal at compile time means adding or dropping a variant fails the build
// here as well as in the tests below.
const _: () = assert!(
    MatcherName::COUNT == 25,
    "the value-matcher vocabulary is closed at 25"
);

#[cfg(test)]
mod tests {
    use super::*;

    /// The 25 spellings exactly as they appear in the schema `matcherName` enum, in declaration
    /// order — the independent reference the enum is checked against. Kept parallel to
    /// [`MatcherName::ALL`]; the tests assert the two agree element-by-element.
    const SCHEMA_SPELLINGS: [&str; 25] = [
        "toBe",
        "toEqual",
        "toStrictEqual",
        "toBeTruthy",
        "toBeFalsy",
        "toBeNull",
        "toBeUndefined",
        "toBeDefined",
        "toBeNaN",
        "toBeTypeOf",
        "toBeInstanceOf",
        "toBeGreaterThan",
        "toBeGreaterThanOrEqual",
        "toBeLessThan",
        "toBeLessThanOrEqual",
        "toBeCloseTo",
        "toContain",
        "toContainEqual",
        "toHaveLength",
        "toHaveProperty",
        "toMatch",
        "toMatchObject",
        "toBeOneOf",
        "toThrow",
        "toSatisfy",
    ];

    #[test]
    fn every_schema_spelling_round_trips_through_serde() {
        assert_eq!(
            MatcherName::ALL.len(),
            SCHEMA_SPELLINGS.len(),
            "the variant table and the schema-spelling table must be the same length"
        );
        for (variant, spelling) in MatcherName::ALL.iter().zip(SCHEMA_SPELLINGS.iter()) {
            let json = format!("\"{spelling}\"");
            // The schema spelling deserialises to its variant …
            let parsed: MatcherName = serde_json::from_str(&json).unwrap();
            assert_eq!(
                parsed, *variant,
                "{spelling} must deserialise to its variant"
            );
            // … and the variant re-serialises to the identical spelling (a true round-trip).
            let reserialized = serde_json::to_string(&parsed).unwrap();
            assert_eq!(
                reserialized, json,
                "{variant:?} must serialise back to {spelling}"
            );
            // The pure accessor and the serde wire form are independent encodings of the same
            // spelling; asserting they agree pins them both to the schema (paired assertion).
            assert_eq!(
                variant.as_str(),
                *spelling,
                "as_str must match the schema spelling"
            );
        }
    }

    #[test]
    fn vocabulary_is_closed_at_twenty_five() {
        // Pins the count directly (the "dropped matcher" negative space, alongside the compile-time
        // `const _` and the fixed-length `ALL`).
        assert_eq!(
            MatcherName::COUNT,
            25,
            "the value-matcher vocabulary is closed at 25"
        );
        assert_eq!(
            MatcherName::ALL.len(),
            25,
            "ALL must enumerate all 25 variants"
        );
    }

    #[test]
    fn all_spellings_are_distinct() {
        // Negative space: the vocabulary is genuinely 25 distinct names — no two variants collide
        // on a wire spelling, which would make deserialisation ambiguous.
        let mut seen = std::collections::BTreeSet::new();
        for matcher in MatcherName::ALL {
            assert!(
                seen.insert(matcher.as_str()),
                "matcher spelling {} is duplicated",
                matcher.as_str()
            );
        }
        assert_eq!(seen.len(), 25, "all 25 matcher spellings must be distinct");
    }

    #[test]
    fn an_unknown_spelling_is_rejected() {
        // Negative space: the enum is closed — a plausible but non-vocabulary matcher (a mock-only
        // matcher the schema excludes, and pure nonsense) fails to deserialise rather than sneaking
        // in as a fallback variant.
        for unknown in ["toHaveBeenCalled", "resolves", "rejects", "toBeAwesome", ""] {
            let json = format!("\"{unknown}\"");
            let parsed: Result<MatcherName, _> = serde_json::from_str(&json);
            assert!(
                parsed.is_err(),
                "{unknown} must not deserialise into the closed vocabulary"
            );
        }
    }
}
