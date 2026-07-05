//! [`FakeChatModel`] — the canned-completion, recording [`ChatModel`] fake.
//!
//! Stands in for `chat-completion` and the `llmRubric` scorer with no provider call: it replays a
//! queue of canned [`ChatResponse`]/[`RunError`] results in order and records every [`ChatRequest`],
//! so a test drives deterministic completions (including a rubric score in the content) and asserts
//! on the exact prompts sent.

use std::collections::VecDeque;
use std::sync::Mutex;

use tmx_core::Milliseconds;
use tmx_core::RunError;
use tmx_core::ports::driven::{ChatModel, ChatRequest, ChatResponse};

/// A [`ChatModel`] that replays canned completions and records the requests it received.
///
/// Completions are consumed FIFO. When the script is empty, a default empty completion echoing the
/// requested model is returned, so an unscripted model still drives a Flow deterministically.
#[derive(Debug, Default)]
pub struct FakeChatModel {
    scripted: Mutex<VecDeque<Result<ChatResponse, RunError>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl FakeChatModel {
    /// An empty model: every call returns the default empty completion until scripted.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next completion to return `content`.
    #[must_use]
    pub fn with_completion(self, content: impl Into<String>) -> Self {
        self.push_result(Ok(ChatResponse {
            content: content.into(),
            model: String::new(),
            prompt_tokens: None,
            completion_tokens: None,
            ms: Milliseconds(0),
        }));
        self
    }

    /// Script the next completion to fail with `error`.
    #[must_use]
    pub fn with_error(self, error: RunError) -> Self {
        self.push_result(Err(error));
        self
    }

    /// Enqueue one scripted result (shared-reference form).
    pub fn push_result(&self, result: Result<ChatResponse, RunError>) {
        if let Ok(mut queue) = self.scripted.lock() {
            queue.push_back(result);
        }
    }

    /// The requests this model was asked to complete, in call order.
    #[must_use]
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl ChatModel for FakeChatModel {
    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, RunError> {
        let model = request.model.clone();
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        let scripted = self.scripted.lock().ok().and_then(|mut q| q.pop_front());
        scripted.unwrap_or_else(|| {
            Ok(ChatResponse {
                content: String::new(),
                model,
                prompt_tokens: None,
                completion_tokens: None,
                ms: Milliseconds(0),
            })
        })
    }
}
