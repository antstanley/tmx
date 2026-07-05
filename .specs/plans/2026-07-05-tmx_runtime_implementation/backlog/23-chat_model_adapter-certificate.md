# Done Certificate — Task 23: Chat model adapter (`chat-completion` and `llmRubric`)

**Task:** [23-chat_model_adapter.md](23-chat_model_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 23. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 23) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `ChatCompletionsModel` is the `chat-completion` executor and the backend for the `llmRubric` scorer, against the ChatCompletions spec.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Wiring `ChatCompletionsModel` into the Task 17 composition root (`crates/tmx-cli/src/compose.rs`), behind its feature, must not change the existing `exec`/`assert` run path; the Task 19 `eval` path's `matcher`/`exec` scorer behaviour must stay unchanged.

## Obligations

- **O1 — A `chat-completion` task calls the model and merges the completion into state; the `llmRubric` scorer produces a normalized score consumed by `eval`.**
  - *Claim:* a `chat-completion` task calls the model through the `ChatModel` port and merges the completion into state; the `llmRubric` scorer, backed by the same port, produces a normalized `[0,1]` score that `eval` consumes.
  - *Evidence to collect:* read `crates/tmx-adapters/src/chat.rs`; run the tests using the `FakeChatModel` in `crates/tmx-testkit` — a normal completion merged into state, and an `eval` (the eval path in `crates/tmx-core/src/fanout.rs`) with an `llmRubric` scorer whose fake judge response parses to a normalized score consumed by the scorecard.
  - *Checks:* resolve both the `chat-completion` task and the `llmRubric` scorer to the same `ChatModel` port → `ChatCompletionsModel`; trace that the judge response is parsed into a score in `[0,1]` before `eval`'s weighted mean.
  - *Status:* ☐ unverified

- **O2 — A non-conforming `llmRubric` judge response is a typed `RunFailure` rather than a zero, and an API failure is typed rather than a panic (negative space).**
  - *Claim:* a non-conforming `llmRubric` judge response is a typed `RunFailure`, not a silent zero; a transport/API failure is a typed `RunError`, not a panic.
  - *Evidence to collect:* run the malformed-judge-response test (`FakeChatModel` scripted with a non-conforming response → expect `RunFailure`, not `0.0`) and an API-failure test (→ expect a typed `RunError`); read `crates/tmx-adapters/src/chat.rs` for the transport/API-error→`RunError` translation and the bounded captured response.
  - *Checks:* trace that a non-conforming judge response yields a `RunFailure` at the scorer boundary rather than defaulting to zero.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; run the adapter's tests with its feature enabled. Confirm the captured-response cap is a named constant (`CAPTURED_OUTPUT_MAX_BYTES`) in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☐ unverified

- **O4 — Run a `chat-completion` flow and an `eval` with an `llmRubric` scorer over the fake model and confirm the completion and the score (Reviewable).**
  - *Claim:* a reviewer can run a `chat-completion` flow and an `eval` with an `llmRubric` scorer over the `FakeChatModel` and observe the completion merged into state and the normalized score in the scorecard.
  - *Evidence to collect:* run the reviewable `chat-completion` flow and the `llmRubric` eval over the fake model; observe the merged completion and the normalized score in the `Scorecard`.
  - *Status:* ☐ unverified

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- This task consumes the Task 19 `eval` path; backing `llmRubric` through the `ChatModel` port must not change the `matcher`/`exec` scorer paths or the summary aggregation — spot-check a mixed-scorer `eval` still behaves.
- The adapter is behind a Cargo feature — build with it enabled to exercise the tests. Real-model tests would be `#[ignore]`; the deterministic coverage is the `FakeChatModel`.
- Confirm the request shape is taken from the `chat-completion` task config per the ChatCompletions spec.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
