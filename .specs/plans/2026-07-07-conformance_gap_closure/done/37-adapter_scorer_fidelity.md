# Task 37 — Adapter/scorer fidelity (store timeout + llmRubric endpoint)

**Plan:** [plan.md](../plan.md) · **Certificate:** [37-adapter_scorer_fidelity-certificate.md](37-adapter_scorer_fidelity-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Concurrency (per-task timeout), §ObjectStore; [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §Scorers (llmRubric)
**Depends on:** —
**Produces:** the `store` task honours the per-task `timeout` under the same cancellation contract as `exec`/`run`/`fetch`; the `llmRubric` scorer honours its `apiUrl`/`apiKey` (routes the judge call to the configured endpoint) — both exercised end-to-end.
**Pointers:** `crates/tmx-core/src/dispatch.rs` (where `exec`/`run`/`fetch` timeouts are set at dispatch; `store` op currently carries none), `crates/tmx-core/src/ports/driven.rs` (`ObjectStore`/`StoreOp` — thread a timeout), `crates/tmx-adapters/src/store.rs` (apply the per-request timeout), `crates/tmx-core/src/fanout.rs` (`score_one` builds a `ChatRequest` with only model/messages/temperature/max_tokens), `crates/tmx-core/src/ports/driven.rs` (`ChatRequest` — add url/key override), `crates/tmx-adapters/src/chat.rs` (use the per-request endpoint/key).

## Steps

- [x] Thread a per-task `timeout` into the `store` dispatch path and the `ObjectStore`/`StoreOp` port so a `store` task times out under the cancellation contract exactly like `fetch` (typed `task_timeout`). Reuse the existing named timeout limit; no new bound.
- [x] Thread the `llmRubric` scorer's `apiUrl`/`apiKey` (from the Scorer schema) into the `ChatRequest` / `ChatModel` call so a rubric judge can target a configured endpoint/key; when absent, fall back to the composed default (today's behaviour).
- [x] Add tests: a `store` task against a slow endpoint surfaces `task_timeout` at ~its timeout; an `llmRubric` scorer with an `apiUrl` routes the judge request to that URL (observed against a local server); absent `apiUrl`/`apiKey` still uses the default.

## Definition of done

- [x] A `store` task honours its per-task `timeout` (typed `task_timeout` on breach), consistent with `exec`/`run`/`fetch`.
- [x] The `llmRubric` scorer's `apiUrl`/`apiKey` route the judge call; absent them, the composed default is used.
- [x] Meets the repo definition of done (tests incl. negative space, `cargo fmt`/`clippy -D warnings`/`nextest`/`scripts/purity.sh` clean; reuse named limits).
- [x] Reviewable: `tmx run` a `store` flow with a short `timeout` against an unreachable/slow endpoint and observe typed `task_timeout`; run an `eval` with an `llmRubric` scorer pointing `apiUrl` at a local server and observe the judge request hit that server.
