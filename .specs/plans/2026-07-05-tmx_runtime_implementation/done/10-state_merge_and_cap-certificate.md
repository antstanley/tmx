# Done Certificate — Task 10: State merge, normalization, and the size cap

**Task:** [10-state_merge_and_cap.md](10-state_merge_and_cap.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

> This certificate is a verification protocol for Task 10. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 10) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion. O4 is the
Reviewable item; record DONE only when O1…O4 are all SATISFIED.

## Premises

- **P1 — Goal.** Produce the pure state-merge function with incremental canonical-JSON byte
  accounting and the hard `STATE_SIZE_MAX_BYTES` cap enforced as a typed error and an assertion.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This is a new pure-core unit. It builds on the Task 04 `PipelineState`/`Value`
  types in `crates/tmx-core/src/model.rs`, the `RunError` type, and the Task 02 constants
  `STATE_SIZE_MAX_BYTES`/`JSON_DEPTH_MAX` in `tmx-schema::limits`; those must keep compiling and
  passing. No task calls `merge` yet — Task 11's runner loop will — so there is no in-scope caller to
  regress.

## Obligations

- **O1 — A merge writes under the resolved key, a text result wraps as `message` and a byte result as `blob`, and the incremental size equals a full canonical re-serialization for representative states.**
  - *Claim:* normalization turns a non-JSON adapter result into `{ "message": … }` for valid UTF-8
    text and `{ "blob": <base64> }` for bytes before the merge; `merge(state, key, output)` writes
    `state[output ?? name] = output`; and the incrementally tracked canonical-JSON byte length equals
    a wholesale canonical re-serialization of the resulting state for representative inputs.
  - *Evidence to collect:* read the `merge` module (new file under `crates/tmx-core/src/`, e.g.
    `merge.rs`) and `PipelineState` in `crates/tmx-core/src/model.rs`; run
    `cargo nextest run -p tmx-core merge` and confirm the write-under-key test, the text-to-`message`
    and bytes-to-`blob` normalization tests, and the test asserting the incremental size equals
    `serde_json` canonical re-serialization for several representative states all pass.
  - *Checks:* trace one merge and confirm the incremental byte delta added equals the canonical
    serialization length of the added subtree (UTF-8, no insignificant whitespace), including the
    overwrite case where a key already exists and the delta can be negative.
  - *Status:* ☑ SATISFIED — Read `merge.rs` and `model.rs`. `normalize_output` matches 01 §Runtime
    entities and 04 step 5: `Json` passes through, `Text`→`{message}`, `Bytes`→`{message}` when valid
    UTF-8 else `{blob:<base64>}` (base64 hand-rolled, pinned to the RFC 4648 §10 vectors and traced by
    hand for `f`→`Zg==` and `0xFFFE`→`//4=`). `merge` writes `state[key]=output` under the resolved
    `output ?? name` key. Traced the incremental accounting: insert adds `key_token + 1 + value_len`
    (+1 comma iff the map is already non-empty), overwrite swaps `old_len` for `value_len` — a total
    that is order-independent, so it equals `serde_json`'s render regardless of key ordering; no `u64`
    underflow (state size always exceeds any single value's length by the braces+key). Tests
    `merge_writes_under_the_resolved_key_and_overwrites`, `normalize_passes_json_through_and_wraps_text_as_message`,
    `normalize_wraps_non_utf8_bytes_as_base64_blob`, `base64_matches_the_rfc4648_vectors`,
    `incremental_size_equals_a_wholesale_reserialization` all PASS. Independently property-checked
    `size_bytes() == serde_json::to_string(as_value()).len()` across 24 000 randomized merges (scalars,
    strings, nested arrays/objects, blobs, overwrites) — held at every step; a `debug_assert_eq!`
    backstops the same equality on every production merge.

- **O2 — An over-cap merge returns `state_cap_exceeded` naming the offending task, verified by at/above-cap boundary tests.**
  - *Claim:* a merge that would push the serialized state over `STATE_SIZE_MAX_BYTES` returns a
    `RunFailure` with code `state_cap_exceeded` naming the offending task (an input-reachable typed
    error, not only a panic), also asserted as a backstop; boundary tests cover one below / at / one
    above the cap.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core merge` and confirm the below-cap merge
    returns `Ok`, the at-cap and above-cap merges return `RunFailure` `state_cap_exceeded`, and the
    error value carries the offending task name; confirm no panic on the input-reachable over-cap
    path (the assertion is a backstop, not the reported error).
  - *Checks:* resolve the over-cap branch and confirm it constructs the typed `RunFailure` carrying
    the task name before (or independently of) any `assert!` backstop, so a real workload gets the
    clean typed error rather than an abort.
  - *Status:* ☑ SATISFIED — Resolved the over-cap branch (merge.rs:276-292): it computes `new_size`
    from the pre-computed delta *before* mutating the state, and on `new_size > cap_bytes` constructs
    `RunError::run_failure("state_cap_exceeded", …).with_task(task)` and returns it. There is no
    `assert!` on this path at all — only a non-panicking `debug_assert!` that the branch and the error
    agree — so a real over-cap workload gets the clean typed error, and the state is left untouched
    (rejected before the `map.insert`). `merge_below_at_and_above_the_cap` PASS: below-cap `Ok` at
    cap-1, at-cap `Ok` at exactly the cap (bound is `<=`), above-cap `Err` with `code ==
    state_cap_exceeded`, `category == RunFailure`, `task == "uploader"`, size unchanged at 2.
    `default_cap_is_the_hard_ceiling_and_cannot_be_widened` PASS proves `new()` wires the real
    `STATE_SIZE_MAX_BYTES` and `with_cap` clamps down, never widening past the ceiling. Independently
    reproduced: an over-cap merge returned `RunFailure [state_cap_exceeded]: merging task "packer"
    would grow the Pipeline state to 208 bytes, over the 64-byte cap`, state left `{}`.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the cap and depth bounds are
    the named `tmx-schema::limits` constants `STATE_SIZE_MAX_BYTES`/`JSON_DEPTH_MAX`, not magic
    numbers; run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no
    match) confirming `tmx-core` takes on no async-runtime/I/O edge.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0 (clean); `cargo clippy --all-targets
    --all-features -- -D warnings` exit 0 (clean; the over-cap/unreachable arms use a `let-else`
    typed-error fallback, not a denied `unwrap`/`expect`); `cargo nextest run` = 100 passed / 0
    skipped. The only bounds are the named `tmx-schema::limits` constants `STATE_SIZE_MAX_BYTES` and
    `JSON_DEPTH_MAX`; the base64 alphabet/group/pad are named `const`s and the 256-byte test cap /
    RFC vectors are test-local — no magic bound literals in production code. Purity: `cargo tree -p
    tmx-core -i tokio` reports no such package (tokio is absent from the whole graph) and
    `scripts/purity.sh` exits 0 ("tmx-schema, tmx-core, tmx-testkit carry no I/O or async dependency
    edge").

- **O4 — Reviewable: run the merge + cap tests and confirm the reported over-cap error names the task and the incremental size matches a wholesale re-serialization (Reviewable).**
  - *Claim:* a reviewer can run the merge and cap tests and observe the over-cap error names the
    task, and the incremental size matches a wholesale re-serialization.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core merge` and read the summary; confirm the
    over-cap test asserts the task name inside the error and the size-equivalence test asserts
    incremental-equals-wholesale, with zero failures.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core merge` = 9 passed / 0 failed. The over-cap
    test (`merge_below_at_and_above_the_cap`) asserts `err.task.as_deref() == Some("uploader")` inside
    the error; the size-equivalence test (`incremental_size_equals_a_wholesale_reserialization`)
    asserts `size_bytes() == canonical_len(as_value())` at every step. Both observed passing, and
    independently reproduced via a throwaway probe (over-cap error names the task; incremental equals
    wholesale over 24 000 random merges) that has since been removed — the tree is back to the
    intended diff.

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The Task 04 `PipelineState`/`Value`
  types and the Task 02 `STATE_SIZE_MAX_BYTES`/`JSON_DEPTH_MAX` constants this unit imports must
  still compile and pass.

## Residue

- The Steps require rejecting a document nested deeper than `JSON_DEPTH_MAX` at merge, but DoD items
  1/2 do not name it explicitly; confirm a `json_too_deep` (`ValidationError`) path exists and is
  tested at the depth boundary, or record it as a gap.
  — RESOLVED: `check_merge_depth` (merge.rs:335-365) rejects an over-deep merged document as
  `RunError::validation("json_too_deep", …)` with the task attached, *iteratively* (explicit stack,
  no recursion — cannot stack-overflow on a pathological value), before mutating. It counts the
  output's root at state-depth 2 (the top-level state object is depth 1), i.e. it bounds the *merged*
  document to `JSON_DEPTH_MAX` — the spec-faithful reading of "any merged JSON value". Tested by
  `merge_rejects_a_document_deeper_than_the_depth_cap` (at-cap accepted, one deeper rejected with
  `json_too_deep`/`Validation`, state untouched) and independently reproduced.
- Canonical-JSON reproducibility depends on key ordering and number formatting; confirm the
  incremental accounting and the wholesale re-serialization agree on object key order and integer/
  float rendering, else the size-equivalence test could pass on one state and drift on another.
  — RESOLVED: the per-key delta is `serde_json::to_string(key).len()` for the key token (so escaping
  is serde's, not hand-rolled) plus `serde_json::to_string(value).len()` for the exact sub-`Value`
  that is moved into the map unchanged, so number rendering is identical in isolation and in-state.
  The total is order-independent (sum of per-key contributions + fixed braces/commas), so it agrees
  whether serde renders keys sorted or in insertion order. Confirmed over 24 000 randomized merges
  and backstopped by a `debug_assert_eq!` on every production merge.
- Confirm a `blob` base64 payload and a large `map`/`eval` result array both count toward the cap, as
  04 §State size cap requires.
  — RESOLVED: a `blob` normalizes to a JSON string whose full base64 length is counted by
  `canonical_len`, and arrays are counted the same way (the size delta is `canonical_len(&output)`
  over the whole normalized value). The randomized probe merged both blobs and arrays and the
  incremental count matched the wholesale re-serialization at every step.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED and every residue item resolved. `normalize_output` +
`StateBuilder::merge` implement 01 §Runtime entities and 04 §State size cap exactly: write under the
resolved `output ?? name` key, wrap text as `message` / non-UTF-8 bytes as a base64 `blob`, track the
canonical-JSON byte size incrementally (equal to a wholesale re-serialization — verified over 24 000
randomized merges and a per-merge `debug_assert_eq!`), and reject over-cap merges as a typed
`RunFailure`/`state_cap_exceeded` naming the task (constructed and returned before any assert, state
left untouched) and over-deep merges as a typed `ValidationError`/`json_too_deep`. Bounds are the
named `tmx-schema::limits` constants; `with_cap` clamps down and never widens past the ceiling. Gates
all green from the repo root: `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -D
warnings`, `cargo nextest run` (100/100), and `scripts/purity.sh` (tmx-core keeps no I/O/async edge).
Greenfield — no in-scope caller to regress; the imported Task 02/04 types still compile and pass. The
temporary verifier probe was removed; the tree is back to the intended two-file diff and builds green.
