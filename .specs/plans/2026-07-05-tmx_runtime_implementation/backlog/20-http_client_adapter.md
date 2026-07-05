# Task 20 — HTTP client adapter (`fetch`)

**Plan:** [plan.md](../plan.md) · **Certificate:** [20-http_client_adapter-certificate.md](20-http_client_adapter-certificate.md)

**Implements:** [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Executor ports (`HttpClient`); [development-guidelines.md](../../../development-guidelines.md) §Defensive coding (Third-party API boundary)
**Depends on:** 05, 17
**Produces:** `ReqwestHttpClient` — the `fetch` executor with method, headers, query, body/`bodyType`, `followRedirects`, bounded retries, and a per-task timeout
**Pointers:** `crates/tmx-adapters/src/http.rs` (new), `crates/tmx-cli/src/compose.rs` (wire into the bundle)

## Steps

- [ ] Implement `fetch` behind the `HttpClient` port: method, headers, query, request body with `bodyType`, `followRedirects`, and a per-task `timeout`.
- [ ] Bound retries by a named units-last constant in `tmx-schema::limits` (e.g. `FETCH_RETRIES_MAX`; no unbounded retry), and bound the captured response body by `CAPTURED_OUTPUT_MAX_BYTES`; treat the response as adversarial — validate status and shape, never deserialize-and-trust.
- [ ] Translate every `reqwest` error into a typed `RunError` at the boundary; wire the adapter into the composition root.
- [ ] Add tests (against a local test server or the fake) for a 2xx body, a non-2xx, a redirect, a timeout, and a retry exhaustion.

## Definition of done

- [ ] A `fetch` task performs a real request honouring method/headers/query/body and the per-task timeout, and a redirect is followed only when `followRedirects` is set.
- [ ] Retries are bounded by the named constant, an oversized body returns `output_too_large`, and a host/transport failure is a typed `RunError` rather than a panic (negative space).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run a flow with a `fetch` task against a local server and confirm the response body in state, the bounded retry, and the timeout behaviour.
