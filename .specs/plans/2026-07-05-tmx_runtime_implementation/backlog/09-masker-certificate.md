# Done Certificate — Task 09: Masker (structural secret redaction)

**Task:** [09-masker.md](09-masker.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 09. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 09) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion. O4 is the
Reviewable item; record DONE only when O1…O4 are all SATISFIED.

## Premises

- **P1 — Goal.** Produce the domain-policy Masker — a sensitive-value registry plus a value-based
  redactor that scrubs every value leaving the core, including within nested JSON.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This is a new pure-core unit. It builds on the Task 04 `Value`/`RunError`
  types and the Task 02 constant `MASK_SCAN_LEN_MIN_BYTES` in `tmx-schema::limits`; those must keep
  compiling and passing. No output port routes through the Masker yet — Task 11 wires the runner's
  output boundary and Task 26 the reporters — so there is no in-scope caller to regress.

## Obligations

- **O1 — A secret echoed inside a nested-JSON task output is redacted in the emitted value; a registered value `>= MASK_SCAN_LEN_MIN_BYTES` is redacted by substring, and a shorter one only on exact match.**
  - *Claim:* the redactor scans an emitted `Value`/string against every registered sensitive value
    and redacts occurrences at any nesting depth of a JSON document; a registered value of length
    `>= MASK_SCAN_LEN_MIN_BYTES` (default 6) is redacted wherever it appears as a substring, while a
    value shorter than the floor is redacted only on an exact whole-value match.
  - *Evidence to collect:* read the registry and redactor in `crates/tmx-core/src/mask.rs`; run
    `cargo nextest run -p tmx-core mask` and confirm the nested-JSON redaction test (a secret buried
    inside an object/array is scrubbed in the output), a substring test for a `>=`-floor value, and
    an exact-match-only test for a below-floor value all pass.
  - *Checks:* trace a secret value through the redactor and confirm the scan-floor branch taken for a
    below-`MASK_SCAN_LEN_MIN_BYTES` (e.g. 5-byte) value is the exact-match arm, not the substring
    scan; trace a `>=`-floor value and confirm it takes the substring-scan arm and recurses into
    nested JSON rather than comparing only the top-level value.
  - *Status:* ☐ unverified

- **O2 — A leaked-secret test fails closed — an emission that bypasses the Masker trips the boundary assertion — and a below-floor value does not clobber an unrelated substring.**
  - *Claim:* the paired boundary assertions hold: the registry is asserted populated before any
    output port runs, and an output port asserts it routed its payload through the Masker, so an
    emission that bypasses the Masker aborts (negative space); and a below-floor sensitive value does
    not redact an unrelated substring that merely happens to contain those bytes.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core mask` and confirm a leaked-secret test
    that constructs a bypassing emission trips the boundary assertion (fails closed, not a silent
    pass), and a non-clobber test where a short registered value overlaps unrelated text leaves that
    text intact.
  - *Checks:* resolve the two boundary-assertion sites and confirm they are independent paths (one on
    the registry-populated side, one on the output-port side), so neither alone is load-bearing;
    trace the below-floor path and confirm the exact-match guard prevents a substring hit on
    unrelated text.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the scan floor is the named
    `tmx-schema::limits` constant `MASK_SCAN_LEN_MIN_BYTES`, not a magic `6`; run the `cargo tree`
    purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no match) confirming `tmx-core`
    takes on no async-runtime/I/O edge.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the masking tests, including the nested-JSON leak test and the below-floor non-clobber test (Reviewable).**
  - *Claim:* a reviewer can run the masking test suite and observe the nested-JSON leak test and the
    below-floor non-clobber test pass.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core mask` and read the summary; confirm the
    run includes the nested-JSON leak case and the below-floor non-clobber case from O1/O2 and
    reports zero failures.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The Task 04 `Value`/`RunError` types
  and the Task 02 `MASK_SCAN_LEN_MIN_BYTES` constant this unit imports must still compile and pass.

## Residue

- Overlapping registered values (one a substring of another, or a short value contained in a long
  one) — confirm redaction order does not leave a partial leak; this is inside item 1's "every
  registered value" but is the subtle case.
- Confirm base64 `blob` values and stringified numbers are also scanned, not only plain string
  fields, so a secret re-encoded on the way out is still caught.
- The allocation-light hot-path claim (borrowed data, pre-sized buffers) is a Step, not a DoD item;
  note it for review but it does not gate.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
