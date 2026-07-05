//! [`SeededIdGenerator`] — the deterministic UUIDv7 [`IdGenerator`] fake.
//!
//! The run-id determinism seam. The production adapter mints a fresh time-random UUIDv7 per run;
//! this fake mints a **reproducible** sequence from a fixed seed, so two fresh generators built from
//! the same seed hand out byte-identical ids. Each id is a well-formed UUIDv7 (version nibble `7`,
//! RFC 9562 variant), with a monotonically increasing timestamp prefix so a lexical sort of the
//! sequence stays chronological — the same ordering property the real `RunStore` relies on.

use std::sync::atomic::{AtomicU64, Ordering};

use tmx_core::RunId;
use tmx_core::ports::driven::IdGenerator;

/// The default seed a [`SeededIdGenerator::new`] draws its random tail from.
///
/// A deterministic test fixture (not an engine limit), so it lives here rather than in
/// `tmx-schema::limits`. Any two generators sharing a seed emit the same id sequence.
const DEFAULT_SEED: u64 = 0x7359_6d78_5f74_6d78;

/// The default 48-bit millisecond origin the first id's timestamp prefix encodes. A fixed,
/// arbitrary instant (2022-01-01T00:00:00Z in Unix milliseconds) kept well inside 48 bits.
const DEFAULT_ORIGIN_UNIX_MS: u64 = 1_640_995_200_000;

/// The 48-bit mask isolating the UUIDv7 timestamp field (the leading six bytes).
const TIMESTAMP_MASK_48: u64 = 0x0000_FFFF_FFFF_FFFF;

/// A deterministic UUIDv7 [`IdGenerator`]: id `n` is a pure function of `(seed, n)`.
///
/// The internal counter starts at zero and increments per [`new_run_id`](IdGenerator::new_run_id).
/// The timestamp prefix is `origin_ms + counter`, so the sequence is monotonically increasing and
/// lexically sortable; the remaining bytes are a `splitmix64` hash of `(seed, counter)`, so the ids
/// look distinct without a `uuid` dependency. Interior mutability keeps the port's `&self` signature.
#[derive(Debug)]
pub struct SeededIdGenerator {
    seed: u64,
    origin_ms: u64,
    counter: AtomicU64,
}

impl SeededIdGenerator {
    /// A generator seeded with [`DEFAULT_SEED`] from the [`DEFAULT_ORIGIN_UNIX_MS`] origin.
    #[must_use]
    pub fn new() -> Self {
        Self::seeded(DEFAULT_SEED)
    }

    /// A generator with a caller-chosen `seed` from the default origin.
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self {
            seed,
            origin_ms: DEFAULT_ORIGIN_UNIX_MS,
            counter: AtomicU64::new(0),
        }
    }
}

impl Default for SeededIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdGenerator for SeededIdGenerator {
    fn new_run_id(&self) -> RunId {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let text = format_uuid_v7(self.seed, self.origin_ms, n);
        // The bytes are laid out to satisfy the UUIDv7 pattern by construction (see `format_uuid_v7`
        // and the `every_generated_id_is_a_valid_uuid_v7` test that proves it over a long run), so
        // this construction cannot fail. `unwrap_or_else(unreachable!)` documents that invariant
        // without an `unwrap`/`expect` panic path in library code.
        RunId::new(text)
            .unwrap_or_else(|_| unreachable!("a seeded UUIDv7 is well-formed by layout"))
    }
}

/// A `splitmix64` step — a fast, deterministic bit mix used only to fill the id's random tail.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Lay out one deterministic UUIDv7 for `(seed, counter)` as a lowercase-hyphenated string.
///
/// Bytes 0..6 carry the 48-bit `origin_ms + counter` timestamp (big-endian, monotonic); bytes 6..16
/// carry a `splitmix64` hash of `(seed, counter)`. The version nibble (byte 6 high) is forced to `7`
/// and the variant nibble (byte 8 high) into `{8,9,a,b}`, so the result matches the `RunId` pattern.
fn format_uuid_v7(seed: u64, origin_ms: u64, counter: u64) -> String {
    let ts48 = origin_ms.wrapping_add(counter) & TIMESTAMP_MASK_48;
    let hi = splitmix64(seed ^ counter);
    let lo = splitmix64(hi.wrapping_add(counter));

    let mut bytes = [0u8; 16];
    // 48-bit timestamp prefix, big-endian: the top two bytes of the shifted u64 are zero.
    bytes[0..6].copy_from_slice(&(ts48 << 16).to_be_bytes()[0..6]);
    bytes[6..14].copy_from_slice(&hi.to_be_bytes());
    bytes[14..16].copy_from_slice(&(lo as u16).to_be_bytes());

    // Version 7 in the high nibble of byte 6; RFC 9562 variant (10xx) in the high nibble of byte 8.
    bytes[6] = (bytes[6] & 0x0F) | 0x70;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;

    let mut out = String::with_capacity(36);
    for (i, b) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        // Two lowercase hex digits per byte — the alphabet the RunId pattern requires.
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0F) as u32, 16).unwrap_or('0'));
    }
    out
}
