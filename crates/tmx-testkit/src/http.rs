//! [`FakeHttpClient`] — the canned-response, recording [`HttpClient`] fake.
//!
//! Stands in for `fetch` with no socket: it replays a queue of canned [`HttpResponse`]/[`RunError`]
//! results in order and records every [`HttpRequest`], so a test drives deterministic responses and
//! asserts on the exact requests made.

use std::collections::VecDeque;
use std::sync::Mutex;

use indexmap::IndexMap;
use tmx_core::Milliseconds;
use tmx_core::RunError;
use tmx_core::ports::driven::{HttpClient, HttpRequest, HttpResponse};

/// An [`HttpClient`] that replays canned responses and records the requests it received.
///
/// Responses are consumed FIFO. When the script is empty, a default `200` with an empty body is
/// returned, so an unscripted client still drives a Flow deterministically.
#[derive(Debug, Default)]
pub struct FakeHttpClient {
    scripted: Mutex<VecDeque<Result<HttpResponse, RunError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttpClient {
    /// An empty client: every call returns the default `200` until responses are scripted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next call to return `status` with `body`.
    #[must_use]
    pub fn with_response(self, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.push_result(Ok(HttpResponse {
            status,
            headers: IndexMap::new(),
            body: body.into(),
            ms: Milliseconds(0),
        }));
        self
    }

    /// Script the next call to fail with `error`.
    #[must_use]
    pub fn with_error(self, error: RunError) -> Self {
        self.push_result(Err(error));
        self
    }

    /// Enqueue one scripted result (shared-reference form).
    pub fn push_result(&self, result: Result<HttpResponse, RunError>) {
        if let Ok(mut queue) = self.scripted.lock() {
            queue.push_back(result);
        }
    }

    /// The requests this client was asked to send, in call order.
    #[must_use]
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl HttpClient for FakeHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RunError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        let scripted = self.scripted.lock().ok().and_then(|mut q| q.pop_front());
        scripted.unwrap_or_else(|| {
            Ok(HttpResponse {
                status: 200,
                headers: IndexMap::new(),
                body: Vec::new(),
                ms: Milliseconds(0),
            })
        })
    }
}
