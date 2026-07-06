//! The `reqwest`-backed [`ChatCompletionsModel`] adapter — the `chat-completion` executor and the
//! backend for the `llmRubric` scorer.
//!
//! [`ChatCompletionsModel`] implements the `ChatModel` port
//! ([`.specs/06-ports-and-adapters.md` §Executor ports](../../../../.specs/06-ports-and-adapters.md)):
//! it POSTs an OpenAI-shaped **ChatCompletions** request (`{ model, messages, temperature?,
//! max_tokens? }`) to the configured endpoint with a `Bearer` API key, reads the response in bounded
//! chunks, and parses `choices[0].message.content` into a [`ChatResponse`]. The same port backs the
//! `llmRubric` scorer, so a judge call and a `chat-completion` task cross identical machinery.
//!
//! The response is treated as **adversarial** (the same discipline the `fetch` adapter follows): the
//! body is read in bounded chunks and rejected the moment it exceeds
//! [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES) (never buffered whole),
//! a non-2xx status is a typed `chat_api_error` carrying a *bounded* slice of the response body, and a
//! body that is not a conforming ChatCompletions object is a typed `chat_bad_response`. Every
//! `reqwest`/transport failure is translated into a typed [`RunError`] at the boundary via
//! [`From<ChatError>`](RunError) — a host/API failure is data, **never** a panic
//! (06 §Adapters return typed errors, never panic on host failure).
//!
//! **The API key stays maskable.** It is held on the adapter, used only to *sign* a request (the
//! `Authorization` header) and never embedded in a [`ChatResponse`] or a captured error body (the
//! error captures the *response*, never the request), so no emitted payload carries the raw key. This
//! module lives behind the `chat` Cargo feature so a minimal build can drop it (and its
//! `reqwest`/`tokio` edge).

use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;

use tmx_core::error::RunError;
use tmx_core::model::Milliseconds;
use tmx_core::ports::driven::{ChatModel, ChatRequest, ChatResponse};
use tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES;

/// The environment variable naming the full ChatCompletions endpoint URL.
const ENDPOINT_ENV: &str = "TMX_CHAT_API_URL";
/// The environment variable naming the API key sent as a `Bearer` token.
const API_KEY_ENV: &str = "TMX_CHAT_API_KEY";

/// The `chat-completion` executor: a `reqwest`-backed [`ChatModel`] adapter against an OpenAI-shaped
/// ChatCompletions endpoint.
///
/// Holds the endpoint URL, the optional API key it signs with, and the captured-output cap it
/// enforces (default [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES);
/// tests construct one with a tiny cap to exercise the `output_too_large` path with a small payload).
#[derive(Debug, Clone)]
pub struct ChatCompletionsModel {
    /// The `reqwest` client used for every completion.
    client: reqwest::Client,
    /// The full ChatCompletions endpoint URL to POST to. Empty means "not configured": a call is a
    /// typed `chat_no_endpoint` error rather than a request to nowhere.
    endpoint: String,
    /// The API key sent as `Authorization: Bearer <key>`, when configured. Never emitted in a result.
    api_key: Option<String>,
    /// The captured response-body ceiling, in bytes.
    output_cap_bytes: u64,
}

impl ChatCompletionsModel {
    /// A model targeting `endpoint`, signing with `api_key` when present, bounding captured bodies by
    /// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`chat_client_init_failed`) if the underlying `reqwest` client
    /// cannot be built (e.g. the TLS backend fails to initialise).
    pub fn new(endpoint: impl Into<String>, api_key: Option<String>) -> Result<Self, RunError> {
        Self::with_output_cap_bytes(endpoint, api_key, CAPTURED_OUTPUT_MAX_BYTES)
    }

    /// A model with an explicit captured-body cap, in bytes — for tests exercising the
    /// `output_too_large` path with a small, fast payload.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`chat_client_init_failed`) if the `reqwest` client cannot be built.
    pub fn with_output_cap_bytes(
        endpoint: impl Into<String>,
        api_key: Option<String>,
        output_cap_bytes: u64,
    ) -> Result<Self, RunError> {
        let client = reqwest::Client::builder().build().map_err(|error| {
            RunError::run_failure(
                "chat_client_init_failed",
                format!("failed to build the chat-completion client: {error}"),
            )
        })?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            api_key: api_key.filter(|key| !key.is_empty()),
            output_cap_bytes,
        })
    }

    /// A model configured from the environment: [`TMX_CHAT_API_URL`](ENDPOINT_ENV) is the endpoint and
    /// [`TMX_CHAT_API_KEY`](API_KEY_ENV) the key. Both are optional — an absent endpoint yields a model
    /// whose calls fail with a typed `chat_no_endpoint`, so composing the adapter never fails a run
    /// that does not use `chat-completion`.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`chat_client_init_failed`) if the `reqwest` client cannot be built.
    pub fn from_env() -> Result<Self, RunError> {
        let endpoint = std::env::var(ENDPOINT_ENV).unwrap_or_default();
        let api_key = std::env::var(API_KEY_ENV).ok();
        Self::new(endpoint, api_key)
    }

    /// Perform `request`, returning the completion or an adapter-local [`ChatError`].
    ///
    /// Builds the OpenAI-shaped body, POSTs it with the `Bearer` key, then reads the response in
    /// bounded chunks (rejected the moment it exceeds the cap), maps a non-2xx status to a typed API
    /// error carrying a bounded body slice, and parses `choices[0].message.content` into the response.
    async fn complete_inner(&self, request: ChatRequest) -> Result<ChatResponse, ChatError> {
        if self.endpoint.is_empty() {
            return Err(ChatError::NoEndpoint);
        }

        // Serialise the OpenAI-shaped body by hand and set the JSON content type — `reqwest`'s `.json()`
        // helper needs its `json` feature, which this crate deliberately does not enable, so the body
        // is a plain `serde_json` string over the same minimal `reqwest` stack the `fetch` adapter uses.
        let body = serde_json::to_vec(&build_request_body(&request)).map_err(|error| {
            ChatError::BadRequest {
                detail: error.to_string(),
            }
        })?;
        let mut builder = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .body(body);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }

        let started = Instant::now();
        let response = builder.send().await.map_err(ChatError::Transport)?;
        let status = response.status().as_u16();

        // Read the body in bounded chunks: a response larger than the cap is `output_too_large` before
        // it is ever fully buffered — the adversarial-response contract.
        let mut raw = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(ChatError::Body)? {
            raw.extend_from_slice(&chunk);
            if raw.len() as u64 > self.output_cap_bytes {
                return Err(ChatError::OutputTooLarge {
                    cap_bytes: self.output_cap_bytes,
                });
            }
        }

        // A non-2xx is an API failure, not a completion: surface the status and a bounded slice of the
        // response body (never the whole thing — an error body is as adversarial as a success body).
        if !(200..300).contains(&status) {
            return Err(ChatError::Api {
                status,
                body: bounded_lossy(&raw, self.output_cap_bytes),
            });
        }

        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        parse_response(&raw, &request.model, Milliseconds(ms))
    }
}

#[async_trait]
impl ChatModel for ChatCompletionsModel {
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, RunError> {
        self.complete_inner(request).await.map_err(RunError::from)
    }
}

/// Build the OpenAI-shaped ChatCompletions request body from the port's [`ChatRequest`].
///
/// [`tmx_schema::ChatMessage`] is `Deserialize`-only, so each message is assembled by hand into a JSON
/// object (`role`, `content`, and `name` only when set); `temperature`/`max_tokens` are emitted only
/// when the request carries them.
fn build_request_body(request: &ChatRequest) -> Value {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            let mut object = serde_json::Map::new();
            object.insert("role".to_string(), Value::String(message.role.clone()));
            object.insert("content".to_string(), message.content.clone());
            if let Some(name) = &message.name {
                object.insert("name".to_string(), Value::String(name.clone()));
            }
            Value::Object(object)
        })
        .collect();

    let mut body = serde_json::Map::new();
    body.insert("model".to_string(), Value::String(request.model.clone()));
    body.insert("messages".to_string(), Value::Array(messages));
    if let Some(temperature) = request.temperature
        && let Some(number) = serde_json::Number::from_f64(temperature)
    {
        body.insert("temperature".to_string(), Value::Number(number));
    }
    if let Some(max_tokens) = request.max_tokens {
        body.insert("max_tokens".to_string(), Value::Number(max_tokens.into()));
    }
    Value::Object(body)
}

/// Parse a ChatCompletions response body into a [`ChatResponse`].
///
/// Extracts `choices[0].message.content` (a string) as the completion; a body that does not carry one
/// is a typed `chat_bad_response`. The response `model` falls back to the requested model when absent,
/// and `usage.prompt_tokens`/`usage.completion_tokens` are captured when present.
fn parse_response(
    raw: &[u8],
    request_model: &str,
    ms: Milliseconds,
) -> Result<ChatResponse, ChatError> {
    let value: Value = serde_json::from_slice(raw).map_err(|error| ChatError::BadResponse {
        detail: format!("the response body is not JSON: {error}"),
    })?;

    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| ChatError::BadResponse {
            detail: "the response has no `choices[0].message.content` string".to_string(),
        })?
        .to_string();

    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(request_model)
        .to_string();

    let usage = value.get("usage");
    let prompt_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(as_u32);
    let completion_tokens = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(as_u32);

    Ok(ChatResponse {
        content,
        model,
        prompt_tokens,
        completion_tokens,
        ms,
    })
}

/// Read a JSON number as a `u32`, when it fits (a token count).
fn as_u32(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok())
}

/// A UTF-8-lossy rendering of at most `cap_bytes` of `raw` — the bounded body slice a `chat_api_error`
/// carries, so an adversarial error body can never blow the message size.
fn bounded_lossy(raw: &[u8], cap_bytes: u64) -> String {
    let take = usize::try_from(cap_bytes)
        .unwrap_or(usize::MAX)
        .min(raw.len());
    String::from_utf8_lossy(&raw[..take]).into_owned()
}

/// The internal, adapter-local failure modes of a chat completion.
///
/// Kept private and translated to a typed [`RunError`] at the port boundary via
/// [`From<ChatError>`](RunError) — the same discipline the HTTP and process adapters use, so every
/// host/API failure funnels through one typed translation, never a panic.
#[derive(Debug)]
enum ChatError {
    /// No endpoint was configured — a call cannot be made.
    NoEndpoint,
    /// The request body could not be serialised (a broken message value).
    BadRequest {
        /// What was wrong with the request.
        detail: String,
    },
    /// A transport failure sending the request or awaiting the response.
    Transport(reqwest::Error),
    /// A failure while reading the response body.
    Body(reqwest::Error),
    /// The endpoint returned a non-2xx status; `body` is a bounded slice of the response.
    Api {
        /// The HTTP status code returned.
        status: u16,
        /// A bounded, UTF-8-lossy slice of the response body.
        body: String,
    },
    /// The response body was not a conforming ChatCompletions object.
    BadResponse {
        /// What was wrong with the body.
        detail: String,
    },
    /// The response body exceeded the adapter's captured-output cap.
    OutputTooLarge {
        /// The cap that was exceeded, in bytes.
        cap_bytes: u64,
    },
}

impl From<ChatError> for RunError {
    fn from(error: ChatError) -> Self {
        match error {
            ChatError::NoEndpoint => RunError::run_failure(
                "chat_no_endpoint",
                format!("no chat-completion endpoint is configured (set `{ENDPOINT_ENV}`)"),
            ),
            ChatError::BadRequest { detail } => RunError::run_failure(
                "chat_bad_request",
                format!("the chat-completion request could not be built: {detail}"),
            ),
            ChatError::Transport(source) => {
                if source.is_timeout() {
                    RunError::run_failure(
                        "task_timeout",
                        format!("the chat-completion request exceeded its timeout: {source}"),
                    )
                } else {
                    RunError::run_failure(
                        "chat_request_failed",
                        format!("the chat-completion request failed: {source}"),
                    )
                }
            }
            ChatError::Body(source) => RunError::run_failure(
                "chat_request_failed",
                format!("failed to read the chat-completion response body: {source}"),
            ),
            ChatError::Api { status, body } => RunError::run_failure(
                "chat_api_error",
                format!("the chat-completion endpoint returned status {status}: {body}"),
            ),
            ChatError::BadResponse { detail } => RunError::run_failure(
                "chat_bad_response",
                format!("the chat-completion response was not conforming: {detail}"),
            ),
            ChatError::OutputTooLarge { cap_bytes } => RunError::run_failure(
                "output_too_large",
                format!("the chat-completion response body exceeded the {cap_bytes}-byte cap"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};

    use tmx_core::error::ErrorCategory;
    use tmx_schema::ChatMessage;

    /// A one-message request to `model`.
    fn request(model: &str, content: &str) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: Value::String(content.to_string()),
                name: None,
            }],
            temperature: Some(0.2),
            max_tokens: Some(64),
        }
    }

    /// Write an HTTP/1.1 response with `body` and `Connection: close`.
    fn respond(stream: &mut TcpStream, status_line: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    }

    /// Read a request off `stream`, returning `(request_body, authorization_header)`.
    fn read_request(stream: &TcpStream) -> (String, String) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();

        let mut content_length = 0_usize;
        let mut authorization = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().unwrap_or(0);
                } else if name.eq_ignore_ascii_case("authorization") {
                    authorization = value.trim().to_string();
                }
            }
        }
        let mut body = vec![0_u8; content_length];
        reader.read_exact(&mut body).unwrap();
        (String::from_utf8_lossy(&body).into_owned(), authorization)
    }

    /// Spawn a one-shot server that captures the request into `captured` and replies with
    /// `status_line` + `reply_body`. Returns the endpoint URL to POST to.
    fn spawn_once(
        status_line: &'static str,
        reply_body: &'static [u8],
        captured: std::sync::Arc<std::sync::Mutex<Option<(String, String)>>>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Some(Ok(mut stream)) = listener.incoming().next() {
                let seen = read_request(&stream);
                *captured.lock().unwrap() = Some(seen);
                respond(&mut stream, status_line, reply_body);
            }
        });
        format!("http://{addr}/v1/chat/completions")
    }

    #[test]
    fn chat_errors_map_to_typed_run_errors_never_panics() {
        // Every host/API-failure mode is a RunFailure with its own stable, snake_case code — never a
        // panic (negative space). A missing endpoint, an API status, a malformed body, and an over-cap
        // body each map to a distinct, meaningful code.
        let no_endpoint: RunError = ChatError::NoEndpoint.into();
        assert_eq!(no_endpoint.category, ErrorCategory::RunFailure);
        assert_eq!(no_endpoint.code, "chat_no_endpoint");

        let api: RunError = ChatError::Api {
            status: 429,
            body: "rate limited".to_string(),
        }
        .into();
        assert_eq!(api.code, "chat_api_error", "a non-2xx is a typed API error");
        assert!(
            api.message.contains("429"),
            "the status is carried in the message: {}",
            api.message
        );

        let over_cap: RunError = ChatError::OutputTooLarge { cap_bytes: 8 }.into();
        assert_eq!(over_cap.code, "output_too_large", "the over-cap limit code");
        assert_eq!(over_cap.category, ErrorCategory::RunFailure);
    }

    #[test]
    fn build_request_body_carries_model_messages_and_sampling() {
        // The outgoing body is the OpenAI shape: model, an ordered messages array, and the sampling
        // knobs only when set.
        let body = build_request_body(&request("gpt-x", "hello"));
        assert_eq!(body["model"], Value::String("gpt-x".to_string()));
        assert_eq!(
            body["messages"][0]["content"],
            Value::String("hello".to_string()),
            "the message content reaches the body"
        );
        assert_eq!(
            body["max_tokens"],
            Value::Number(64.into()),
            "max_tokens set"
        );
        assert!(
            body.get("temperature").is_some(),
            "temperature is emitted when set"
        );
    }

    #[tokio::test]
    async fn a_2xx_completion_is_parsed_and_the_bearer_key_is_sent() {
        // A conforming ChatCompletions response is parsed into the completion, model, and token usage;
        // the API key rides the Authorization header and never appears in the parsed response.
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let reply = br#"{"model":"gpt-x-2","choices":[{"message":{"role":"assistant","content":"the answer"}}],"usage":{"prompt_tokens":11,"completion_tokens":5}}"#;
        let url = spawn_once("200 OK", reply, std::sync::Arc::clone(&captured));

        let model = ChatCompletionsModel::new(url, Some("secret-key".to_string()))
            .expect("the client builds");
        let response = model
            .complete(request("gpt-x", "hi"))
            .await
            .expect("a conforming response parses");

        assert_eq!(response.content, "the answer", "the completion is parsed");
        assert_eq!(
            response.model, "gpt-x-2",
            "the response model overrides the request model"
        );
        assert_eq!(
            response.prompt_tokens,
            Some(11),
            "the prompt token usage is captured"
        );

        let (sent_body, authorization) =
            captured.lock().unwrap().clone().expect("a request arrived");
        assert_eq!(
            authorization, "Bearer secret-key",
            "the API key rides the Authorization header"
        );
        assert!(
            sent_body.contains("\"model\":\"gpt-x\""),
            "the request model reached the server: {sent_body}"
        );
    }

    #[tokio::test]
    async fn a_non_2xx_status_is_a_typed_api_error_not_a_completion() {
        // A 500 is an API failure the caller sees as a typed error, never an Err-less empty completion
        // and never a panic (negative space).
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let url = spawn_once("500 Internal Server Error", b"upstream boom", captured);
        let model = ChatCompletionsModel::new(url, None).expect("the client builds");
        let error = model
            .complete(request("gpt-x", "hi"))
            .await
            .expect_err("a 5xx is a typed error");
        assert_eq!(error.category, ErrorCategory::RunFailure, "a run failure");
        assert_eq!(error.code, "chat_api_error", "with the API-error code");
        assert!(
            error.message.contains("500"),
            "the status is named: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn a_malformed_2xx_body_is_a_typed_bad_response() {
        // A 200 whose body carries no `choices[0].message.content` is a typed `chat_bad_response`, not
        // a silent empty completion (negative space).
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let url = spawn_once("200 OK", br#"{"choices":[]}"#, captured);
        let model = ChatCompletionsModel::new(url, None).expect("the client builds");
        let error = model
            .complete(request("gpt-x", "hi"))
            .await
            .expect_err("a non-conforming body is a typed error");
        assert_eq!(error.code, "chat_bad_response", "the malformed-body code");
        assert_eq!(error.category, ErrorCategory::RunFailure, "a run failure");
    }

    #[tokio::test]
    async fn an_oversized_body_is_output_too_large() {
        // A response larger than the cap is rejected as `output_too_large` rather than buffered whole —
        // the adversarial-response bound.
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let big = br#"{"model":"m","choices":[{"message":{"role":"assistant","content":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}}]}"#;
        let url = spawn_once("200 OK", big, captured);
        let model =
            ChatCompletionsModel::with_output_cap_bytes(url, None, 8).expect("the client builds");
        let error = model
            .complete(request("gpt-x", "hi"))
            .await
            .expect_err("an over-cap body is rejected");
        assert_eq!(error.code, "output_too_large", "the named limit code");
    }

    #[tokio::test]
    async fn a_missing_endpoint_is_a_typed_error_not_a_panic() {
        // A model with no configured endpoint fails closed with a typed `chat_no_endpoint`, never a
        // request to nowhere or a panic — so composing the adapter never breaks a run that skips chat.
        let model = ChatCompletionsModel::new(String::new(), None).expect("the client builds");
        let error = model
            .complete(request("gpt-x", "hi"))
            .await
            .expect_err("an unconfigured endpoint is a typed error");
        assert_eq!(error.code, "chat_no_endpoint", "names the missing endpoint");
        assert_eq!(error.category, ErrorCategory::RunFailure, "a run failure");
    }

    #[tokio::test]
    async fn a_transport_failure_is_typed_not_a_panic() {
        // A connection refused (no listener) is a typed RunError, never a panic (negative space).
        let model =
            ChatCompletionsModel::new("http://127.0.0.1:1/v1/chat/completions".to_string(), None)
                .expect("the client builds");
        let error = model
            .complete(request("gpt-x", "hi"))
            .await
            .expect_err("a connection failure is a typed error");
        assert_eq!(
            error.code, "chat_request_failed",
            "the transport-failure code"
        );
        assert_eq!(error.category, ErrorCategory::RunFailure, "a run failure");
    }
}
