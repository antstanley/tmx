# Done Certificate — Task 09: Masker (structural secret redaction)

**Task:** [09-masker.md](09-masker.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

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
  - *Status:* ☑ SATISFIED — Registry + redactor read (mask.rs): `register` (133-160) buckets by the
    `MASK_SCAN_LEN_MIN_BYTES` floor (142-146); `scrub_value` (265-310) recurses into `Array` (281-291)
    and `Object` (292-307); `scrub_str` (316-332) does exact-then-substring. `cargo nextest run -p
    tmx-core mask` → 15/15 pass, incl. `redacts_secret_buried_in_nested_json` (secret at object→array→
    object depth is scrubbed), `above_floor_secret_redacts_as_substring` ("prefix-[REDACTED]-suffix"),
    `below_floor_secret_redacts_on_exact_match_only`. Trace: a 4-byte value ("abcd" < 6) is pushed to
    the `exact` bucket (mask.rs:144-145) and matched only by `input == secret` (mask.rs:319-323) — the
    exact arm; a `>=`-floor value ("sk-abcdef123456") goes to `substring` and is matched by
    `current.contains(secret)` (mask.rs:326-330) while `scrub_value` recurses. Mutation: forcing the
    floor to 1 (below-floor → substring bucket) makes `below_floor_secret_redacts_on_exact_match_only`
    FAIL (`"[REDACTED]ef gradebook"`), proving the exact-only arm is load-bearing; reverted, tree green.

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
  - *Status:* ☑ SATISFIED — Two independent sites: `assert_ready` (mask.rs:234-249) checks the
    registry-populated side (aggregate emptiness + per-secret `contains`); `assert_routed`
    (mask.rs:256-262) checks the output-port side (`payload.origin == self.id`) — neither references
    the other. `Masked` fields are private (mask.rs:60-63) so a bypassing payload cannot even be
    constructed to reach a `Masked<T>`-typed port; the runtime check is defence-in-depth on top.
    `assert_routed_trips_on_a_bypassing_emission` (`#[should_panic]` "not routed through this Masker")
    passes; MUTATION weakening `assert_routed` to compare `self.id` to itself makes that test FAIL to
    trip (guard load-bearing), reverted. `assert_ready_trips_on_empty_registry_with_resolved_secrets`
    and `assert_ready_trips_when_a_resolved_secret_is_unregistered` exercise the aggregate and
    per-secret paths independently; positive `assert_ready_accepts_*` / `assert_routed_accepts_*`
    confirm no false trips. Non-clobber: `below_floor_secret_redacts_on_exact_match_only` asserts
    `"abcdef gradebook"` is left intact; the floor→1 mutation above proves that assertion load-bearing.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the scan floor is the named
    `tmx-schema::limits` constant `MASK_SCAN_LEN_MIN_BYTES`, not a magic `6`; run the `cargo tree`
    purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no match) confirming `tmx-core`
    takes on no async-runtime/I/O edge.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0; `cargo clippy --all-targets
    --all-features -- -D warnings` exit 0, no warnings; `cargo nextest run` → 91 passed, 0 skipped.
    Floor is the named constant `MASK_SCAN_LEN_MIN_BYTES` (imported mask.rs:41, used mask.rs:138) —
    no magic `6`; no new numeric bound (`REDACTED_PLACEHOLDER` is a string marker, not a limit;
    `limits.rs` untouched — diff is only lib.rs + mask.rs). Purity: `scripts/purity.sh` green;
    `cargo tree -p tmx-core -i tokio` and `-i reqwest` → "did not match any packages" (no async/I/O
    edge); no `std::process`. Every core fn carries ≥2 meaningful assertions (`new`, `register`,
    `redact_value`, `redact_line`, `assert_ready`, `assert_routed`), plus a debug postcondition that no
    substring secret survives redaction (mask.rs:195-198, 216-219).

- **O4 — Reviewable: run the masking tests, including the nested-JSON leak test and the below-floor non-clobber test (Reviewable).**
  - *Claim:* a reviewer can run the masking test suite and observe the nested-JSON leak test and the
    below-floor non-clobber test pass.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core mask` and read the summary; confirm the
    run includes the nested-JSON leak case and the below-floor non-clobber case from O1/O2 and
    reports zero failures.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core mask` → "15 tests run: 15 passed, 0
    skipped" (48 filtered). The run includes the nested-JSON leak case
    (`redacts_secret_buried_in_nested_json`) and the below-floor non-clobber case
    (`below_floor_secret_redacts_on_exact_match_only`), plus the blob/message, stringified-number,
    overlapping-secret, and both boundary-assertion families — zero failures.

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
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED. The Masker redacts every sensitive value leaving the core,
recursing to any nesting depth of a JSON document (`scrub_value`), with the spec's scan floor: values
`>= MASK_SCAN_LEN_MIN_BYTES` (the named constant, no magic 6) redact by substring longest-first for
overlap safety, below-floor values redact on exact whole-value match only. Numbers, base64 blobs, and
message strings are scanned as leaves; the hot path borrows (`Cow`) any unchanged subtree. The
fail-closed guarantee runs along two independent, empirically load-bearing paths: `assert_ready`
(registry populated) and `assert_routed` (payload minted by this Masker) — weakening `assert_routed`
made the bypass test stop tripping, and forcing a below-floor value into the substring bucket made the
non-clobber test clobber; both mutations reverted, tree rebuilds green. fmt/clippy(-D warnings)/nextest
all clean (91 passed; mask 15/15), purity gate green, no tokio/reqwest/process/S3 edge, no new numeric
limit. Regression: greenfield — no in-scope caller (Task 11 wires the output boundary); the only
external references are doc/forward comments. Two non-gating notes for the downstream caller (Task 11):
(1) `register` skips an empty secret value but `assert_ready(&["", …])` would panic on `contains("")`
— an asymmetry that fails CLOSED (abort, never leak) and should be handled by filtering empties
symmetrically at the call site; (2) the debug-only leak postcondition would false-fire if a registered
substring secret were itself a ≥6-byte substring of the literal `"[REDACTED]"` (e.g. "REDACT") — a
release-compiled-out, pathological case no real secret meets. Neither touches an O1–O4 obligation or
the spec's guarantee.
