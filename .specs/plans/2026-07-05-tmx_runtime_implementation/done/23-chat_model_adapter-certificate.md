# Done Certificate — Task 23: Chat model adapter (`chat-completion` and `llmRubric`)

**Task:** [23-chat_model_adapter.md](23-chat_model_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

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
  - *Status:* ☑ SATISFIED — `dispatch.rs` (`TaskWith::ChatCompletion` → `ports.chat.complete`, merging `{content, model}`) and `fanout.rs::score_one` (`"llmRubric"` arm) both cross the same `ChatModel` port that `compose.rs` wires to `ChatCompletionsModel` under the `chat` feature. Ran `runner_runs_a_chat_completion_task_and_merges_the_completion_into_state` (asserts the merged `{content, model}` state AND the recorded request), `a_mixed_scorer_eval_emits_a_full_scorecard_and_gates_on_the_threshold`, and `a_threshold_less_eval_only_reports_and_passes` — all PASS; the judge score is parsed to `[0,1]` before the weighted mean.

- **O2 — A non-conforming `llmRubric` judge response is a typed `RunFailure` rather than a zero, and an API failure is typed rather than a panic (negative space).**
  - *Claim:* a non-conforming `llmRubric` judge response is a typed `RunFailure`, not a silent zero; a transport/API failure is a typed `RunError`, not a panic.
  - *Evidence to collect:* run the malformed-judge-response test (`FakeChatModel` scripted with a non-conforming response → expect `RunFailure`, not `0.0`) and an API-failure test (→ expect a typed `RunError`); read `crates/tmx-adapters/src/chat.rs` for the transport/API-error→`RunError` translation and the bounded captured response.
  - *Checks:* trace that a non-conforming judge response yields a `RunFailure` at the scorer boundary rather than defaulting to zero.
  - *Status:* ☑ SATISFIED — ran `a_non_conforming_llm_rubric_response_is_a_run_failure_not_a_zero` (typed `rubric_bad_output`, not 0.0) — PASS. All 8 `chat::tests` run against a real one-shot HTTP server and PASS: non-2xx → `chat_api_error` (status in message), malformed 2xx → `chat_bad_response`, connection refused → `chat_request_failed`, no endpoint → `chat_no_endpoint`, over-cap body → `output_too_large` (rejected mid-stream at cap 8 bytes, never buffered whole). Every failure funnels through the private `ChatError` → `From<ChatError> for RunError` translation; the API-error body is bounded via `bounded_lossy` (and the chunk loop already caps `raw`). No panic path.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; run the adapter's tests with its feature enabled. Confirm the captured-response cap is a named constant (`CAPTURED_OUTPUT_MAX_BYTES`) in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☑ SATISFIED — validator ran independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 265/265 passed; `cargo nextest run --all-features` 282/282 passed (2 skipped = the pre-existing `#[ignore]` live-S3 store tests, not this task); `cargo nextest run -p tmx-adapters --features chat` — all 8 `chat::tests` pass; `cargo build` and `cargo build --all-features` clean; `scripts/purity.sh` green. The cap reuses `tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES` (units-last named constant); no new numeric bound introduced. Env-var names are named constants (`ENDPOINT_ENV`/`API_KEY_ENV`).

- **O4 — Run a `chat-completion` flow and an `eval` with an `llmRubric` scorer over the fake model and confirm the completion and the score (Reviewable).**
  - *Claim:* a reviewer can run a `chat-completion` flow and an `eval` with an `llmRubric` scorer over the `FakeChatModel` and observe the completion merged into state and the normalized score in the scorecard.
  - *Evidence to collect:* run the reviewable `chat-completion` flow and the `llmRubric` eval over the fake model; observe the merged completion and the normalized score in the `Scorecard`.
  - *Status:* ☑ SATISFIED — validator exercised the reviewable action: `cargo nextest run --all-features -E 'test(runner_runs_a_chat_completion_task_and_merges_the_completion_into_state) + test(/rubric/) + test(/eval/)'` — 5/5 PASS. Observed the completion merged as `{"reply": {"content": "the-completion-text", "model": "test-model"}}` and the mixed-scorer eval's scorecard carrying the judge's parsed score with threshold gating intact.

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☑ PRESERVED — default `cargo nextest run` 265/265 including all `cli_run` exec/assert tests and `a_flow_needing_an_unwired_port_exits_five` (which stays live in the default build; it is `#[cfg(not(feature = "chat"))]`-gated so it is compiled out, not failing, when chat is wired). `composes_the_engine_and_advertises_only_real_capabilities` passes in both configurations (Chat absent by default, present under `--all-features`). Mixed-scorer eval (matcher + llmRubric + exec) unchanged and passing.

## Residue

- This task consumes the Task 19 `eval` path; backing `llmRubric` through the `ChatModel` port must not change the `matcher`/`exec` scorer paths or the summary aggregation — spot-check a mixed-scorer `eval` still behaves.
- The adapter is behind a Cargo feature — build with it enabled to exercise the tests. Real-model tests would be `#[ignore]`; the deterministic coverage is the `FakeChatModel`.
- Confirm the request shape is taken from the `chat-completion` task config per the ChatCompletions spec.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED with independently-run evidence and the regression check PRESERVED. `ChatCompletionsModel` correctly implements the `ChatModel` port (OpenAI-shaped body, Bearer signing, bounded chunked reads, `choices[0].message.content` parse), every host/API failure is a typed `RunError` (verified against a real HTTP server: 8/8 adapter tests), the `llmRubric` negative space stays a typed `rubric_bad_output`, and composition mirrors the `store` opt-in pattern exactly (Chat capability advertised only under the `chat` feature). fmt/clippy/nextest all clean: 265/265 default, 282/282 all-features, purity green. Notes (non-blocking): the `task_timeout` translation branch is currently unreachable (no client/request timeout is configured — a latent hook, not a defect); with the `chat` feature on but `TMX_CHAT_API_URL` unset, a chat flow passes preflight and fails at run time with `chat_no_endpoint` — the documented, store-consistent semantics; the implementer's report mis-attributed the 2 all-features skips to the cfg-gated exit-5 test (they are the pre-existing `#[ignore]` live-S3 tests).
