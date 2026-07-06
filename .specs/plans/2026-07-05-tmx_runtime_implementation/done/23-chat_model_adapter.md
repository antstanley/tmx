# Task 23 — Chat model adapter (`chat-completion` and `llmRubric`)

**Plan:** [plan.md](../plan.md) · **Certificate:** [23-chat_model_adapter-certificate.md](23-chat_model_adapter-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Executor ports (`ChatModel`); [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §Scorers (`llmRubric`)
**Depends on:** 05, 17, 19
**Produces:** `ChatCompletionsModel` — the `chat-completion` executor and the backend for the `llmRubric` scorer
**Pointers:** `crates/tmx-adapters/src/chat.rs` (new), `crates/tmx-cli/src/compose.rs` (wire into the bundle)

## Steps

- [x] Implement the `ChatModel` port against the ChatCompletions spec, taking the request shape from the `chat-completion` task config and returning the completion.
- [x] Back the `llmRubric` scorer through the same port, parsing the judge's response into a normalized score in `[0,1]`; a non-conforming response is a `RunFailure`, not a silent zero.
- [x] Translate transport/API errors into typed `RunError`s, bound the captured response, and wire the adapter into the composition root behind its feature.
- [x] Add tests using the fake `ChatModel` for a normal completion and a malformed judge response.

## Definition of done

- [x] A `chat-completion` task calls the model and merges the completion into state, and the `llmRubric` scorer produces a normalized score consumed by `eval`.
- [x] A non-conforming `llmRubric` judge response is a typed `RunFailure` (negative space) rather than a zero, and an API failure is typed rather than a panic.
- [x] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [x] Reviewable: run a `chat-completion` flow and an `eval` with an `llmRubric` scorer over the fake model and confirm the completion and the score.
