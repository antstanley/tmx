//! The `reqwest`-backed [`HttpClient`] adapter — the `fetch` executor.
//!
//! [`ReqwestHttpClient`] implements the `HttpClient` port
//! ([`.specs/06-ports-and-adapters.md` §Executor ports](../../../../.specs/06-ports-and-adapters.md)):
//! it applies the request's method, headers, query, and body, enforces the per-request `timeout`,
//! follows redirects only when the request asks, retries a transport failure a **bounded** number of
//! times ([`FETCH_RETRIES_MAX`](tmx_schema::limits::FETCH_RETRIES_MAX)), and bounds the captured
//! response body by [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
//!
//! The response is treated as **adversarial**: the body is read in bounded chunks and rejected the
//! moment it exceeds the cap (never buffered whole and never deserialise-and-trusted), and every
//! `reqwest` failure is translated into a typed [`RunError`] at the boundary via
//! [`From<HttpError>`](RunError) — a host/transport failure is data, **never** a panic
//! (06 §Adapters return typed errors, never panic on host failure). This is the crate where
//! `reqwest` lives, gated behind the `http` Cargo feature so a minimal build can drop it.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use indexmap::IndexMap;

use tmx_core::error::RunError;
use tmx_core::model::Milliseconds;
use tmx_core::ports::driven::{HttpClient, HttpRequest, HttpResponse};
use tmx_schema::limits::{CAPTURED_OUTPUT_MAX_BYTES, FETCH_RETRIES_MAX};

/// The `fetch` executor: a `reqwest`-backed [`HttpClient`] adapter.
///
/// Holds two clients because `reqwest`'s redirect policy is a *per-client* setting but the port's
/// `follow_redirects` is *per-request*: [`follow`](Self::follow) applies the bounded default policy
/// and [`no_follow`](Self::no_follow) applies [`redirect::Policy::none`](reqwest::redirect::Policy::none),
/// and each call picks the one matching its request. Also holds the captured-output cap it enforces
/// (default [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES)); tests
/// construct one with a tiny cap to exercise the `output_too_large` path with a small payload.
#[derive(Debug, Clone)]
pub struct ReqwestHttpClient {
    /// The client that follows redirects (bounded by `reqwest`'s default policy).
    follow: reqwest::Client,
    /// The client that never follows a redirect (returns the 3xx response verbatim).
    no_follow: reqwest::Client,
    /// The captured response-body ceiling, in bytes.
    output_cap_bytes: u64,
}

impl ReqwestHttpClient {
    /// A client that bounds captured response bodies by
    /// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`http_client_init_failed`) if the underlying `reqwest` clients
    /// cannot be built (e.g. the TLS backend fails to initialise).
    pub fn new() -> Result<Self, RunError> {
        Self::with_output_cap_bytes(CAPTURED_OUTPUT_MAX_BYTES)
    }

    /// A client with an explicit captured-body cap, in bytes — for tests exercising the
    /// `output_too_large` path with a small, fast payload.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`http_client_init_failed`) if a `reqwest` client cannot be built.
    pub fn with_output_cap_bytes(output_cap_bytes: u64) -> Result<Self, RunError> {
        Ok(Self {
            follow: build_client(true)?,
            no_follow: build_client(false)?,
            output_cap_bytes,
        })
    }

    /// Perform `request`, returning the response or an adapter-local [`HttpError`].
    ///
    /// Builds the request once, then makes up to `min(request.retries, FETCH_RETRIES_MAX) + 1`
    /// attempts: a transport failure on `execute` is retried until the bound is reached, then the last
    /// failure is surfaced. Once a response arrives, the body is read in bounded chunks and rejected
    /// the moment it exceeds the cap. No retry happens after a response is in hand.
    async fn send_inner(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| HttpError::InvalidMethod(request.method.clone()))?;
        let client = if request.follow_redirects {
            &self.follow
        } else {
            &self.no_follow
        };

        let mut builder = client.request(method, &request.url);
        for (key, value) in &request.headers {
            builder = builder.header(key, value);
        }
        if !request.query.is_empty() {
            let pairs: Vec<(&String, &String)> = request.query.iter().collect();
            builder = builder.query(&pairs);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }
        if let Some(Milliseconds(ms)) = request.timeout {
            builder = builder.timeout(Duration::from_millis(ms));
        }

        let outgoing = builder.build().map_err(HttpError::Build)?;
        // A task's declared `retries` is clamped to the named ceiling, so a flapping host can never
        // drive unbounded requests — "bounded retries" is literally bounded (Tiger Style).
        let retries = request.retries.min(FETCH_RETRIES_MAX);

        let started = Instant::now();
        let mut attempt = 0_u32;
        let response = loop {
            // A bytes/absent body always clones; only a streaming body (which the port never sends)
            // would fail — surfaced as a typed error, never an `unwrap`.
            let this = outgoing.try_clone().ok_or(HttpError::UncloneableBody)?;
            match client.execute(this).await {
                Ok(response) => break response,
                Err(error) => {
                    if attempt < retries {
                        attempt += 1;
                        continue;
                    }
                    return Err(HttpError::Transport(error));
                }
            }
        };

        let status = response.status().as_u16();
        let mut headers = IndexMap::new();
        for (name, value) in response.headers() {
            if let Ok(text) = value.to_str() {
                headers.insert(name.as_str().to_string(), text.to_string());
            }
        }

        // Read the body in bounded chunks: a response larger than the cap is `output_too_large`
        // before it is ever fully buffered — the adversarial-response contract.
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(HttpError::Body)? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > self.output_cap_bytes {
                return Err(HttpError::OutputTooLarge {
                    cap_bytes: self.output_cap_bytes,
                });
            }
        }

        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(HttpResponse {
            status,
            headers,
            body,
            ms: Milliseconds(ms),
        })
    }
}

#[async_trait]
impl HttpClient for ReqwestHttpClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RunError> {
        self.send_inner(request).await.map_err(RunError::from)
    }
}

/// Build one `reqwest` client with the redirect policy `follow` selects: the bounded default policy
/// (which follows a limited number of redirects, never unbounded) when `true`, or
/// [`redirect::Policy::none`](reqwest::redirect::Policy::none) when `false`.
fn build_client(follow: bool) -> Result<reqwest::Client, RunError> {
    let policy = if follow {
        reqwest::redirect::Policy::default()
    } else {
        reqwest::redirect::Policy::none()
    };
    reqwest::Client::builder()
        .redirect(policy)
        .build()
        .map_err(|error| {
            RunError::run_failure(
                "http_client_init_failed",
                format!("failed to build the HTTP client: {error}"),
            )
        })
}

/// The internal, adapter-local failure modes of an HTTP request.
///
/// Kept private and translated to a typed [`RunError`] at the port boundary via
/// [`From<HttpError>`](RunError) — the same discipline the process adapter uses. It exists so every
/// host failure (a bad method, a build/transport/body error, or an over-cap capture) funnels through
/// one typed translation, never a panic.
#[derive(Debug)]
enum HttpError {
    /// The request `method` was not a valid HTTP method token.
    InvalidMethod(String),
    /// The request could not be built (e.g. an invalid URL or header).
    Build(reqwest::Error),
    /// A transport failure sending the request or awaiting the response (connect, timeout, reset).
    Transport(reqwest::Error),
    /// A failure while reading the response body.
    Body(reqwest::Error),
    /// A bytes body could not be cloned for a retry (a streaming body — the port never sends one).
    UncloneableBody,
    /// The response body exceeded the adapter's captured-output cap.
    OutputTooLarge {
        /// The cap that was exceeded, in bytes.
        cap_bytes: u64,
    },
}

impl From<HttpError> for RunError {
    fn from(error: HttpError) -> Self {
        match error {
            HttpError::InvalidMethod(method) => RunError::run_failure(
                "http_invalid_method",
                format!("`{method}` is not a valid HTTP method"),
            ),
            HttpError::Build(source) => RunError::run_failure(
                "http_request_failed",
                format!("failed to build the HTTP request: {source}"),
            ),
            HttpError::Transport(source) => {
                // A timed-out request shares the `task_timeout` code with the process adapter, so the
                // per-task `timeout` surfaces identically across executors; any other transport
                // failure is a generic, typed request failure.
                if source.is_timeout() {
                    RunError::run_failure(
                        "task_timeout",
                        format!("the HTTP request exceeded its timeout: {source}"),
                    )
                } else {
                    RunError::run_failure(
                        "http_request_failed",
                        format!("the HTTP request failed: {source}"),
                    )
                }
            }
            HttpError::Body(source) => RunError::run_failure(
                "http_request_failed",
                format!("failed to read the HTTP response body: {source}"),
            ),
            HttpError::UncloneableBody => RunError::run_failure(
                "http_request_failed",
                "a streaming request body cannot be retried",
            ),
            HttpError::OutputTooLarge { cap_bytes } => RunError::run_failure(
                "output_too_large",
                format!("the HTTP response body exceeded the {cap_bytes}-byte cap"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tmx_core::error::ErrorCategory;

    /// A bare `GET` request to `url` with no headers/query/body, following redirects, no retries.
    fn get(url: &str) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            url: url.to_string(),
            headers: IndexMap::new(),
            query: IndexMap::new(),
            body: None,
            follow_redirects: true,
            retries: 0,
            timeout: None,
        }
    }

    /// One line of an HTTP/1.1 response with a body and `Connection: close`.
    fn respond(stream: &mut TcpStream, status_line: &str, extra_headers: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    }

    /// Read a request off `stream`, returning `(method, path_with_query, x_test_header, body)`.
    fn read_request(stream: &TcpStream) -> (String, String, String, Vec<u8>) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();

        let mut content_length = 0_usize;
        let mut x_test = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap_or(0);
                } else if name.eq_ignore_ascii_case("x-test") {
                    x_test = value.trim().to_string();
                }
            }
        }
        let mut body = vec![0_u8; content_length];
        reader.read_exact(&mut body).unwrap();
        (method, path, x_test, body)
    }

    /// Spawn a minimal HTTP/1.1 server on an ephemeral loopback port. It handles connections
    /// sequentially (each `Connection: close`), routing on the path:
    /// `/echo` echoes `METHOD PATH` + the request body and reflects the `X-Test` header,
    /// `/redirect` 302s to `/target`, `/target` returns `arrived`, `/big` returns `big_len` bytes,
    /// and `/slow` sleeps past a short client timeout before responding.
    fn spawn_echo(big_len: usize) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (method, path, x_test, body) = read_request(&stream);
                if path.starts_with("/echo") {
                    let mut out = format!("{method} {path}\n").into_bytes();
                    out.extend_from_slice(&body);
                    respond(
                        &mut stream,
                        "200 OK",
                        &format!("x-echo-test: {x_test}\r\n"),
                        &out,
                    );
                } else if path.starts_with("/redirect") {
                    respond(&mut stream, "302 Found", "Location: /target\r\n", b"");
                } else if path.starts_with("/target") {
                    respond(&mut stream, "200 OK", "", b"arrived");
                } else if path.starts_with("/big") {
                    respond(&mut stream, "200 OK", "", &vec![b'x'; big_len]);
                } else if path.starts_with("/slow") {
                    std::thread::sleep(Duration::from_secs(3));
                    respond(&mut stream, "200 OK", "", b"late");
                } else {
                    respond(&mut stream, "404 Not Found", "", b"");
                }
            }
        });
        addr
    }

    /// Spawn a server that accepts a connection, counts it, and drops it without responding — every
    /// request is therefore a transport failure. Used to count bounded retry attempts.
    fn spawn_dropper(counter: Arc<AtomicUsize>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                counter.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        addr
    }

    #[test]
    fn transport_errors_map_to_typed_run_errors() {
        // Every host-failure mode is a RunFailure with its own stable, snake_case code — never a
        // panic. A timeout shares the process adapter's `task_timeout` code; an over-cap body is the
        // named limit code.
        let bad_method: RunError = HttpError::InvalidMethod("BAD METHOD".to_string()).into();
        assert_eq!(bad_method.category, ErrorCategory::RunFailure);
        assert_eq!(bad_method.code, "http_invalid_method");

        let over_cap: RunError = HttpError::OutputTooLarge { cap_bytes: 8 }.into();
        assert_eq!(over_cap.code, "output_too_large", "the over-cap limit code");
        assert_eq!(over_cap.category, ErrorCategory::RunFailure);
    }

    #[tokio::test]
    async fn get_2xx_captures_method_query_headers_and_body() {
        let addr = spawn_echo(0);
        let client = ReqwestHttpClient::new().expect("the client builds");
        let mut request = HttpRequest {
            method: "POST".to_string(),
            url: format!("http://{addr}/echo"),
            headers: IndexMap::new(),
            query: IndexMap::new(),
            body: Some(b"payload-bytes".to_vec()),
            follow_redirects: true,
            retries: 0,
            timeout: None,
        };
        request
            .headers
            .insert("X-Test".to_string(), "hello".to_string());
        request.query.insert("q".to_string(), "42".to_string());

        let response = client.send(request).await.expect("a 2xx request succeeds");
        assert_eq!(response.status, 200, "a 2xx status is returned verbatim");
        let text = String::from_utf8(response.body).unwrap();
        assert!(
            text.starts_with("POST /echo?"),
            "method reached the server: {text:?}"
        );
        assert!(
            text.contains("q=42"),
            "the query reached the server: {text:?}"
        );
        assert!(
            text.ends_with("payload-bytes"),
            "the body reached the server: {text:?}"
        );
        assert_eq!(
            response.headers.get("x-echo-test").map(String::as_str),
            Some("hello"),
            "the request header round-tripped through the server"
        );
    }

    #[tokio::test]
    async fn non_2xx_is_a_typed_result_not_an_error() {
        // A 404 is a real response the caller inspects, not a transport error: the adapter returns
        // it (status + body), never an `Err`.
        let addr = spawn_echo(0);
        let client = ReqwestHttpClient::new().expect("the client builds");
        let response = client
            .send(get(&format!("http://{addr}/missing")))
            .await
            .expect("a non-2xx is Ok with the status, not an Err");
        assert_eq!(
            response.status, 404,
            "the non-2xx status is surfaced as data"
        );
        assert!(
            response.body.is_empty(),
            "the 404 body is captured (empty here)"
        );
    }

    #[tokio::test]
    async fn redirect_followed_only_when_requested() {
        // follow off: the 302 is returned verbatim (Location header, not followed).
        let addr = spawn_echo(0);
        let client = ReqwestHttpClient::new().expect("the client builds");
        let mut request = get(&format!("http://{addr}/redirect"));
        request.follow_redirects = false;
        let response = client
            .send(request)
            .await
            .expect("the redirect response returns");
        assert_eq!(
            response.status, 302,
            "a 302 is not followed when follow is off"
        );
        assert_eq!(
            response.headers.get("location").map(String::as_str),
            Some("/target"),
            "the Location header is surfaced instead of being followed"
        );

        // follow on: the redirect is chased to /target and its 200 body is returned.
        let followed = client
            .send(get(&format!("http://{addr}/redirect")))
            .await
            .expect("the followed redirect resolves");
        assert_eq!(followed.status, 200, "following lands on the 200 target");
        assert_eq!(followed.body, b"arrived", "the target body is captured");
    }

    #[tokio::test]
    async fn oversized_body_is_output_too_large() {
        // A body larger than the cap is rejected as `output_too_large` rather than buffered whole.
        let addr = spawn_echo(64);
        let client = ReqwestHttpClient::with_output_cap_bytes(8).expect("the client builds");
        let error = client
            .send(get(&format!("http://{addr}/big")))
            .await
            .expect_err("an over-cap body is rejected");
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "over-cap is a run failure"
        );
        assert_eq!(error.code, "output_too_large", "with the named limit code");
    }

    #[tokio::test]
    async fn under_cap_body_is_allowed() {
        // Negative-space companion: a body at the cap is captured, not rejected — the bound is a
        // ceiling, not an off-by-one on legitimate output.
        let addr = spawn_echo(8);
        let client = ReqwestHttpClient::with_output_cap_bytes(8).expect("the client builds");
        let response = client
            .send(get(&format!("http://{addr}/big")))
            .await
            .expect("a cap-sized body is allowed");
        assert_eq!(response.status, 200, "the request still succeeds");
        assert_eq!(response.body.len(), 8, "the cap-sized body is captured");
    }

    #[tokio::test]
    async fn timeout_is_a_typed_task_timeout() {
        // A server slower than the per-request timeout surfaces `task_timeout`, promptly.
        let addr = spawn_echo(0);
        let client = ReqwestHttpClient::new().expect("the client builds");
        let mut request = get(&format!("http://{addr}/slow"));
        request.timeout = Some(Milliseconds(100));
        let started = Instant::now();
        let error = client
            .send(request)
            .await
            .expect_err("a slow request times out");
        assert_eq!(error.code, "task_timeout", "the failure names the timeout");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the timeout fired promptly, not after the full server delay"
        );
    }

    #[tokio::test]
    async fn transport_failure_is_typed_not_a_panic() {
        // A connection refused (no listener) is a typed RunError, never a panic (negative space).
        let client = ReqwestHttpClient::new().expect("the client builds");
        // Port 0 is not a listening endpoint; the connect fails.
        let error = client
            .send(get("http://127.0.0.1:1/nowhere"))
            .await
            .expect_err("a connection failure is a typed error");
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "a transport failure is a run failure"
        );
        assert_eq!(
            error.code, "http_request_failed",
            "with the transport-failure code"
        );
    }

    #[tokio::test]
    async fn retries_are_bounded_by_the_named_constant() {
        // A declared retry count under the ceiling makes exactly retries+1 attempts.
        let counter = Arc::new(AtomicUsize::new(0));
        let addr = spawn_dropper(Arc::clone(&counter));
        let client = ReqwestHttpClient::new().expect("the client builds");
        let mut request = get(&format!("http://{addr}/"));
        request.retries = 2;
        let error = client.send(request).await.expect_err("every attempt fails");
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "still a typed error"
        );
        assert_eq!(
            counter.load(Ordering::SeqCst),
            3,
            "two retries means three attempts total"
        );
    }

    #[tokio::test]
    async fn retries_are_clamped_to_the_ceiling() {
        // A declared retry count far above the ceiling is clamped: at most FETCH_RETRIES_MAX+1
        // attempts, never unbounded — the bounded-retry contract.
        let counter = Arc::new(AtomicUsize::new(0));
        let addr = spawn_dropper(Arc::clone(&counter));
        let client = ReqwestHttpClient::new().expect("the client builds");
        let mut request = get(&format!("http://{addr}/"));
        request.retries = u32::MAX;
        let _ = client.send(request).await.expect_err("every attempt fails");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            (FETCH_RETRIES_MAX + 1) as usize,
            "an over-ceiling retry count is clamped to FETCH_RETRIES_MAX+1 attempts"
        );
    }
}
