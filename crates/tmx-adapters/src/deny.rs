//! Denying stub adapters for the not-yet-built executor ports (`fetch` / `file` / `store` /
//! `chat-completion`).
//!
//! Task 17 delivers only the `exec`/`assert` end-to-end path; the HTTP, filesystem, object-store, and
//! chat-model adapters arrive in tasks 20–23. Until then the composition root still has to build the
//! full [`Ports`](tmx_core::ports::driven) bundle, so these stubs stand in for the missing ports. They
//! are **denying**: every call returns an [`ErrorCategory::Environment`] `capability_unavailable`
//! error naming the port. In practice they are never *called* — preflight's capability check
//! (03 §Capability check) rejects a Flow that needs an unwired port up front (exit 5), so a stub is a
//! fail-closed backstop, not a code path a valid run reaches.

use tmx_core::error::{ErrorCategory, RunError};
use tmx_core::ports::driven::{
    ChatModel, ChatRequest, ChatResponse, FileOp, FileResult, FileSystem, HttpClient, HttpRequest,
    HttpResponse, ObjectStore, StoreOp, StoreResult,
};

/// The typed error every denying stub returns — the port is present in the bundle but not wired to a
/// real adapter, so any *call* is a fail-closed [`ErrorCategory::Environment`] error.
fn unavailable(port: &str, task_type: &str) -> RunError {
    RunError::new(
        ErrorCategory::Environment,
        "capability_unavailable",
        format!("the {port} adapter is not wired in this build (`{task_type}` is unavailable)"),
    )
    .with_task(task_type)
}

/// A denying [`HttpClient`] stub — `fetch` is unavailable until task 20.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyingHttpClient;

#[async_trait::async_trait]
impl HttpClient for DenyingHttpClient {
    async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RunError> {
        Err(unavailable("HttpClient", "fetch"))
    }
}

/// A denying [`FileSystem`] stub — `file` is unavailable until task 21.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyingFileSystem;

#[async_trait::async_trait]
impl FileSystem for DenyingFileSystem {
    async fn op(&self, _op: FileOp) -> Result<FileResult, RunError> {
        Err(unavailable("FileSystem", "file"))
    }
}

/// A denying [`ObjectStore`] stub — `store` is unavailable until task 22.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyingObjectStore;

#[async_trait::async_trait]
impl ObjectStore for DenyingObjectStore {
    async fn op(
        &self,
        _op: StoreOp,
        _timeout: Option<tmx_core::model::Milliseconds>,
    ) -> Result<StoreResult, RunError> {
        Err(unavailable("ObjectStore", "store"))
    }
}

/// A denying [`ChatModel`] stub — `chat-completion` is unavailable until task 23.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyingChatModel;

#[async_trait::async_trait]
impl ChatModel for DenyingChatModel {
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, RunError> {
        Err(unavailable("ChatModel", "chat-completion"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll};

    fn block_on_ready<F: Future>(fut: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("a ready future must complete on first poll"),
        }
    }

    #[test]
    fn every_stub_denies_with_an_environment_error_naming_the_port() {
        // Each stub fails closed: an Environment `capability_unavailable` error naming its task type.
        let http = block_on_ready(DenyingHttpClient.send(HttpRequest {
            method: "GET".to_string(),
            url: "https://x".to_string(),
            headers: IndexMap::new(),
            query: IndexMap::new(),
            body: None,
            follow_redirects: false,
            retries: 0,
            timeout: None,
        }))
        .expect_err("the http stub denies");
        assert_eq!(
            http.category,
            ErrorCategory::Environment,
            "environment error"
        );
        assert_eq!(http.code, "capability_unavailable", "the denial code");
        assert_eq!(http.task.as_deref(), Some("fetch"), "names the task type");

        let file = block_on_ready(DenyingFileSystem.op(FileOp::Exists {
            path: "/x".to_string(),
        }))
        .expect_err("the file stub denies");
        assert_eq!(file.code, "capability_unavailable", "file denies too");

        let store = block_on_ready(DenyingObjectStore.op(
            StoreOp::Head {
                key: "k".to_string(),
            },
            None,
        ))
        .expect_err("the store stub denies");
        assert_eq!(store.task.as_deref(), Some("store"), "store names its type");

        let chat = block_on_ready(DenyingChatModel.complete(ChatRequest {
            model: "m".to_string(),
            messages: Vec::new(),
            temperature: None,
            max_tokens: None,
            api_url: None,
            api_key: None,
        }))
        .expect_err("the chat stub denies");
        assert_eq!(
            chat.task.as_deref(),
            Some("chat-completion"),
            "chat names its type"
        );
    }
}
