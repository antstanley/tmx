# Done Certificate — Task 20: HTTP client adapter (`fetch`)

**Task:** [20-http_client_adapter.md](20-http_client_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — discharged by an independent verifier

> This certificate is a verification protocol for Task 20. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 20) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** `ReqwestHttpClient` is the `fetch` executor with method, headers, query, body/`bodyType`, `followRedirects`, bounded retries, and a per-task timeout.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** Wiring `ReqwestHttpClient` into the Task 17 composition root (`crates/tmx-cli/src/compose.rs`) must not change the existing `exec`/`assert` run path — the compose root stays the only place concrete adapter types are named.

## Obligations

- **O1 — A `fetch` task performs a real request honouring method/headers/query/body and the per-task timeout; a redirect is followed only when `followRedirects` is set.**
  - *Claim:* a `fetch` request applies method, headers, query, and request body per `bodyType`, enforces the per-task `timeout`, follows a redirect only when `followRedirects` is set, and merges the response body into state.
  - *Evidence to collect:* read `crates/tmx-adapters/src/http.rs`; run the named tests against a local test server (or the `FakeHttpClient` in `crates/tmx-testkit`) covering a 2xx body that echoes method/headers/query/body, a redirect with `followRedirects` off (expect not followed) and on (expect followed), and a timeout; confirm the response body lands in final state.
  - *Checks:* trace that the `fetch` task routes through the `HttpClient` port to `ReqwestHttpClient` via the `TaskDispatcher`, and that the per-task `timeout` is enforced at the adapter.
  - *Status:* ☑ SATISFIED — adapter tests `get_2xx_captures_method_query_headers_and_body`, `redirect_followed_only_when_requested`, `timeout_is_a_typed_task_timeout` pass against a local std-TCP server (no network). End-to-end: `tmx run` of a two-task fetch flow (GET + POST with an object body) against a local python server merged both response bodies into final state under the task names, exit 0; the POST body arrived as compact JSON with `Content-Type: application/json` (schema-default `bodyType`). A slow endpoint with `timeout: 300ms` failed at ~304ms with the typed `task_timeout` message. Trace: `dispatch.rs` `TaskWith::Fetch` → `build_fetch_request` → `ports.http.send` → `ReqwestHttpClient::send`; the timeout is applied per request via `RequestBuilder::timeout` in the adapter.

- **O2 — Retries are bounded by the named constant, an oversized body returns `output_too_large`, and a host/transport failure is a typed `RunError` rather than a panic (negative space).**
  - *Claim:* retries are bounded by a named constant (no unbounded retry); a response body over `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large`; a host/transport failure is a typed `RunError`, never a panic.
  - *Evidence to collect:* run the negative-space tests — a retry-exhaustion case asserting the retry count stops at the named constant, an over-cap body (expect `output_too_large`), and a connection/transport failure (expect a typed `RunError`); read `crates/tmx-adapters/src/http.rs` for the `reqwest`-error→`RunError` translation at the boundary and the retry-bound constant.
  - *Checks:* confirm the retry bound is a named units-last constant in `tmx-schema::limits` (not a literal) and the body cap is `CAPTURED_OUTPUT_MAX_BYTES`; trace that every `reqwest` error maps to a typed `RunError` at the boundary — no `unwrap`/`expect` on the response, never deserialize-and-trust.
  - *Status:* ☑ SATISFIED — `retries_are_bounded_by_the_named_constant` (retries=2 → exactly 3 server-counted attempts), `retries_are_clamped_to_the_ceiling` (u32::MAX → `FETCH_RETRIES_MAX+1` attempts), `oversized_body_is_output_too_large` + negative-space `under_cap_body_is_allowed`, `transport_failure_is_typed_not_a_panic` (connection refused → typed `http_request_failed`), `transport_errors_map_to_typed_run_errors` all pass. Guard-trip check: the verifier temporarily injected (a) `min(FETCH_RETRIES_MAX + 1)` and (b) an off-by-one `>=` on the body cap — the clamp test and the under-cap negative-space test both FAILED as required, then the injections were reverted (tree byte-identical to the implementer snapshot, `jj diff --from 9c729968` empty, suite green). `FETCH_RETRIES_MAX: u32 = 5` is a named units-last constant in `tmx-schema::limits` with a compile-time sanity assert; the cap defaults to `CAPTURED_OUTPUT_MAX_BYTES` and the body is read in bounded chunks, rejected the moment it exceeds the cap. Every reqwest failure funnels through the private `HttpError` → `From<HttpError> for RunError` (timeout → `task_timeout`, else `http_request_failed`; bad method → `http_invalid_method`; over-cap → `output_too_large`); no `unwrap`/`expect` on any response path. End-to-end: `retries: 99` against a refused port completed clamped in <1s with a typed error, exit 1 — no panic, no unbounded loop.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm the retry bound and body cap are named constants in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☑ SATISFIED — verifier ran independently: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 250/250 passed (10 new http adapter tests + 6 new dispatch bodyType tests); `scripts/purity.sh` green (reqwest confined to `tmx-adapters` behind the `http` feature; pure crates carry no I/O edge). Bounds are named constants in `tmx-schema::limits`. No schema/example changed.

- **O4 — Run a flow with a `fetch` task against a local server and confirm the response body in state, the bounded retry, and the timeout behaviour (Reviewable).**
  - *Claim:* a reviewer can run a flow with a `fetch` task against a local server and observe the response body merged into state, the retry bounded at the named constant on a failing endpoint, and the timeout firing as a typed error on a slow endpoint.
  - *Evidence to collect:* run the reviewable `fetch` flow against a local test server; observe the response body in final state, the retry count capped at the named constant, and the timeout surfacing as a typed error.
  - *Status:* ☑ SATISFIED — the verifier ran three flows via `cargo run -p tmx-cli -- run …` against a local python HTTP server: (1) GET+POST flow → both bodies merged into final state under the task names, exit 0; (2) `timeout: 300ms` against a 5 s endpoint → typed `task_timeout` at ~304ms, exit 1; (3) `retries: 99` against a refused port → clamped, typed `http_request_failed` in <1s, exit 1. The retry cap itself was observed server-side in the adapter tests (`FETCH_RETRIES_MAX+1` attempts counted).

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☑ PRESERVED — the full `tmx-cli` `cli_run` suite (incl. `passing_exec_assert_flow…`) passes; the compose root remains the only place concrete adapter types are named (`ReqwestHttpClient::new()?` replaces `DenyingHttpClient`, `Capability::Http` now advertised); the unwired-port exit-5 test was correctly rebased onto the still-stubbed `file` port.

## Residue

- The "real request" tests need a local test server (e.g. `httpmock`/`wiremock`) or the `FakeHttpClient`; the DoD accepts either. The validator should note which was used and confirm the deterministic path runs without network.
- Global cancellation of an in-flight request is threaded in Task 29 — this task only enforces the per-task `timeout`; do not expect the full grace-period contract here.
- Confirm each `bodyType` variant (json/form/raw) serialises correctly and `followRedirects` defaults per the schema.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☑ DONE — every obligation O1–O4 SATISFIED, regression PRESERVED.
CONFIDENCE: ☑ high
SUMMARY: `ReqwestHttpClient` is a real, wired `fetch` executor: method/headers/query/body-per-`bodyType` honoured (compact JSON + default `Content-Type`, form urlencoding, text/binary raw), per-request timeout enforced and typed as `task_timeout`, redirects followed only when requested (two-client design), retries clamped to the named `FETCH_RETRIES_MAX` (guard-trip verified by fault injection), over-cap bodies rejected as `output_too_large` from bounded chunked reads, and every reqwest failure translated to a typed `RunError` at the boundary. Residue notes: the tests use a dependency-free local std-TCP server (deterministic, offline — no new dev-deps); `bodyType` variants json/form/text/binary all serialise correctly with `followRedirects` defaulting to true per the schema; the per-request `timeout` applies per attempt (a timed-out attempt counts against the bounded retries — total wall time is still bounded by (retries+1)×timeout; full cancellation contract lands in task 29 as planned). Minor non-blocking observations: response headers are captured into an `IndexMap<String,String>` so duplicate or non-UTF-8 header values are lossy (a constraint of the port type, not this adapter), and retries have no backoff (the spec requires only boundedness).
