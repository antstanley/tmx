# Done Certificate — Task 10: State merge, normalization, and the size cap

**Task:** [10-state_merge_and_cap.md](10-state_merge_and_cap.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

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
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the cap and depth bounds are
    the named `tmx-schema::limits` constants `STATE_SIZE_MAX_BYTES`/`JSON_DEPTH_MAX`, not magic
    numbers; run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no
    match) confirming `tmx-core` takes on no async-runtime/I/O edge.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the merge + cap tests and confirm the reported over-cap error names the task and the incremental size matches a wholesale re-serialization (Reviewable).**
  - *Claim:* a reviewer can run the merge and cap tests and observe the over-cap error names the
    task, and the incremental size matches a wholesale re-serialization.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core merge` and read the summary; confirm the
    over-cap test asserts the task name inside the error and the size-equivalence test asserts
    incremental-equals-wholesale, with zero failures.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The Task 04 `PipelineState`/`Value`
  types and the Task 02 `STATE_SIZE_MAX_BYTES`/`JSON_DEPTH_MAX` constants this unit imports must
  still compile and pass.

## Residue

- The Steps require rejecting a document nested deeper than `JSON_DEPTH_MAX` at merge, but DoD items
  1/2 do not name it explicitly; confirm a `json_too_deep` (`ValidationError`) path exists and is
  tested at the depth boundary, or record it as a gap.
- Canonical-JSON reproducibility depends on key ordering and number formatting; confirm the
  incremental accounting and the wholesale re-serialization agree on object key order and integer/
  float rendering, else the size-equivalence test could pass on one state and drift on another.
- Confirm a `blob` base64 payload and a large `map`/`eval` result array both count toward the cap, as
  04 §State size cap requires.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
