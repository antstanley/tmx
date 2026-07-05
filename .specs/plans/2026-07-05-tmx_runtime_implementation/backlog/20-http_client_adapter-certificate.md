# Done Certificate — Task 20: HTTP client adapter (`fetch`)

**Task:** [20-http_client_adapter.md](20-http_client_adapter.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

- **O2 — Retries are bounded by the named constant, an oversized body returns `output_too_large`, and a host/transport failure is a typed `RunError` rather than a panic (negative space).**
  - *Claim:* retries are bounded by a named constant (no unbounded retry); a response body over `CAPTURED_OUTPUT_MAX_BYTES` returns `output_too_large`; a host/transport failure is a typed `RunError`, never a panic.
  - *Evidence to collect:* run the negative-space tests — a retry-exhaustion case asserting the retry count stops at the named constant, an over-cap body (expect `output_too_large`), and a connection/transport failure (expect a typed `RunError`); read `crates/tmx-adapters/src/http.rs` for the `reqwest`-error→`RunError` translation at the boundary and the retry-bound constant.
  - *Checks:* confirm the retry bound is a named units-last constant in `tmx-schema::limits` (not a literal) and the body cap is `CAPTURED_OUTPUT_MAX_BYTES`; trace that every `reqwest` error maps to a typed `RunError` at the boundary — no `unwrap`/`expect` on the response, never deserialize-and-trust.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Confirm the retry bound and body cap are named constants in `tmx-schema::limits`. No schema or example changed → `scripts/validate.sh` is not required.
  - *Status:* ☐ unverified

- **O4 — Run a flow with a `fetch` task against a local server and confirm the response body in state, the bounded retry, and the timeout behaviour (Reviewable).**
  - *Claim:* a reviewer can run a flow with a `fetch` task against a local server and observe the response body merged into state, the retry bounded at the named constant on a failing endpoint, and the timeout firing as a typed error on a slow endpoint.
  - *Evidence to collect:* run the reviewable `fetch` flow against a local test server; observe the response body in final state, the retry count capped at the named constant, and the timeout surfacing as a typed error.
  - *Status:* ☐ unverified

## Regression check

- Wiring into the Task 17 compose root; trace that the existing exec/assert run path (Task 17 test) still passes : ☐ (PRESERVED / REGRESSION)

## Residue

- The "real request" tests need a local test server (e.g. `httpmock`/`wiremock`) or the `FakeHttpClient`; the DoD accepts either. The validator should note which was used and confirm the deterministic path runs without network.
- Global cancellation of an in-flight request is threaded in Task 29 — this task only enforces the per-task `timeout`; do not expect the full grace-period contract here.
- Confirm each `bodyType` variant (json/form/raw) serialises correctly and `followRedirects` defaults per the schema.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
