# Task 10 — State merge, normalization, and the size cap

**Plan:** [plan.md](../plan.md) · **Certificate:** [10-state_merge_and_cap-certificate.md](10-state_merge_and_cap-certificate.md)

**Implements:** [04-execution-engine.md](../../../04-execution-engine.md) §State size cap, §Pipeline execution algorithm (merge step), §Invariants & assertions (state); [01-domain-model.md](../../../01-domain-model.md) §Runtime entities (output normalisation)
**Depends on:** 04
**Produces:** the pure state-merge function with incremental canonical-JSON byte accounting and the hard `STATE_SIZE_MAX_BYTES` cap enforced as a typed error and an assertion
**Pointers:** `crates/tmx-core/src/model.rs` (`PipelineState`), a new `merge` module in `crates/tmx-core/src/`

## Steps

- [x] Implement output normalization: a non-JSON adapter result becomes `{ "message": … }` for valid UTF-8 text and `{ "blob": <base64> }` for bytes, before the merge, so state stays JSON objects all the way down.
- [x] Implement `merge(state, key, output)` writing `state[output ?? name] = output`, asserting the state is an object and the key is a non-empty string on entry.
- [x] Track the serialized canonical-JSON byte length (UTF-8, no insignificant whitespace) incrementally at each merge, and reject a merge that would exceed `STATE_SIZE_MAX_BYTES` with a `RunFailure` (`state_cap_exceeded`) naming the task, also asserted as a backstop.
- [x] Reject a document nested deeper than `JSON_DEPTH_MAX` at merge, and add tests one below / at / one above the state cap.

## Definition of done

- [x] A merge writes under the resolved key, a text result wraps as `message` and a byte result as `blob`, and the incremental size equals a full canonical re-serialization for representative states.
- [x] An over-cap merge returns `state_cap_exceeded` naming the offending task (input-reachable typed error, not only a panic), verified by at/above-cap boundary tests (negative space).
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run the merge + cap tests and confirm the reported over-cap error names the task and the incremental size matches a wholesale re-serialization.
