//! [`Uuidv7Generator`] — the production UUIDv7 [`IdGenerator`] adapter.
//!
//! The real-time counterpart of the testkit's `SeededIdGenerator`: it mints a fresh, time-ordered
//! UUIDv7 per run via the `uuid` crate. UUIDv7 embeds a millisecond timestamp in its leading bytes,
//! so a lexical sort of run ids is chronological — the ordering property the `RunStore` (task 27)
//! relies on to list runs without a separate sort key.

use tmx_core::RunId;
use tmx_core::ports::driven::IdGenerator;

/// Mints a fresh UUIDv7 [`RunId`] per run.
#[derive(Debug, Clone, Copy, Default)]
pub struct Uuidv7Generator;

impl Uuidv7Generator {
    /// A fresh generator. Stateless: two instances behave identically.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl IdGenerator for Uuidv7Generator {
    fn new_run_id(&self) -> RunId {
        // `uuid` renders lowercase-hyphenated with version nibble 7 and an RFC 9562 variant nibble,
        // which is exactly the `RunId` pattern — so construction cannot fail. `unwrap_or_else` with
        // `unreachable!` documents that invariant without an `unwrap`/`expect` panic path.
        let generated = uuid::Uuid::now_v7().to_string();
        RunId::new(generated)
            .unwrap_or_else(|_| unreachable!("a fresh UUIDv7 always matches the RunId pattern"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generated_id_is_a_well_formed_uuid_v7() {
        // A batch of fresh ids are all valid RunIds (the generator never mints a malformed id).
        let generator = Uuidv7Generator::new();
        let mut ids = Vec::new();
        for _ in 0..64 {
            let id = generator.new_run_id();
            assert_eq!(
                id.as_str().len(),
                36,
                "each id is a 36-char hyphenated UUID"
            );
            ids.push(id.as_str().to_string());
        }
        // Version nibble (index 14) is '7' and the variant nibble (index 19) is one of {8,9,a,b}.
        for id in &ids {
            let bytes = id.as_bytes();
            assert_eq!(bytes[14], b'7', "the version nibble is 7 for {id}");
            assert!(
                matches!(bytes[19], b'8' | b'9' | b'a' | b'b'),
                "the variant nibble is in {{8,9,a,b}} for {id}"
            );
        }
    }

    #[test]
    fn ids_are_time_ordered_and_distinct() {
        // Successive ids from one generator are distinct and lexically non-decreasing (UUIDv7 embeds
        // a monotonic millisecond timestamp), the chronological-sort property the RunStore relies on.
        let generator = Uuidv7Generator::new();
        let first = generator.new_run_id().as_str().to_string();
        let second = generator.new_run_id().as_str().to_string();
        assert_ne!(first, second, "two fresh ids are distinct");
        assert!(
            second >= first,
            "a later id sorts at or after an earlier one ({first} <= {second})"
        );
    }
}
