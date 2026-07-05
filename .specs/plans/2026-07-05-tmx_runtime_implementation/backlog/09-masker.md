# Task 09 — Masker (structural secret redaction)

**Plan:** [plan.md](../plan.md) · **Certificate:** [09-masker-certificate.md](09-masker-certificate.md)

**Implements:** [04-execution-engine.md](../../../04-execution-engine.md) §Secrets & masking; [08-errors-and-observability.md](../../../08-errors-and-observability.md) §Masking at the boundary
**Depends on:** 04, 06
**Produces:** the domain-policy Masker — a sensitive-value registry plus a value-based redactor that scrubs every value leaving the core, including within nested JSON
**Pointers:** `crates/tmx-core/src/mask.rs` (new)

## Steps

- [ ] Implement the registry that records every resolved secret value as sensitive, and a redactor that scans an emitted `Value`/string for each registered value, redacting occurrences including inside nested JSON.
- [ ] Apply the scan floor: values shorter than `MASK_SCAN_LEN_MIN_BYTES` (default 6) are redacted on exact match only, not by substring scan, so a short secret cannot clobber unrelated text.
- [ ] Provide the boundary assertions: the registry is asserted populated before any output port runs, and an output port asserts it routed its payload through the Masker (the paired negative-space guarantee).
- [ ] Keep the hot path allocation-light (borrowed data, pre-sized buffers) since it runs on every emission.

## Definition of done

- [ ] A secret echoed inside a nested-JSON task output is redacted in the emitted value; a registered value `>= MASK_SCAN_LEN_MIN_BYTES` is redacted by substring, and a shorter one only on exact match.
- [ ] A leaked-secret test fails closed — an emission that bypasses the Masker trips the boundary assertion (negative space) — and a below-floor value does not clobber an unrelated substring.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the masking tests, including the nested-JSON leak test and the below-floor non-clobber test.
