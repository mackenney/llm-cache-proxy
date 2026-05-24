//! MockUpstream — HTTP mock server for simulating LLM provider APIs.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use bytes::Bytes;
use tokio::task::JoinHandle;

/// A recorded incoming request.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: HeaderMap,
    pub body: Bytes,
}

/// A response to queue for MockUpstream.
#[derive(Clone, Debug)]
pub enum MockResponse {
    /// JSON response with status and body.
    Json { status: u16, body: String },
    /// SSE response with status and chunks (each chunk is a complete SSE frame).
    Sse { status: u16, chunks: Vec<String> },
    /// Error response with status and body.
    Error { status: u16, body: String },
}

struct Inner {
    responses: Mutex<VecDeque<MockResponse>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

/// Mock HTTP server for testing proxy behavior.
///
/// # Example
/// ```ignore
/// let mock = MockUpstream::builder()
///     .json(200, r#"{"content":"hello"}"#)
///     .build()
///     .await;
/// println!("Mock running at {}", mock.url());
/// ```
pub struct MockUpstream {
    addr: SocketAddr,
    handle: Option<JoinHandle<()>>,
    inner: Arc<Inner>,
}

impl MockUpstream {
    /// Start building a MockUpstream with queued responses.
    pub fn builder() -> MockUpstreamBuilder {
        MockUpstreamBuilder::new()
    }

    async fn start(responses: VecDeque<MockResponse>) -> Self {
        let inner = Arc::new(Inner {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        });

        let state = inner.clone();
        let app = Router::new()
            .route("/{*path}", any(handle_request))
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("get local addr");

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        Self {
            addr,
            handle: Some(handle),
            inner,
        }
    }

    /// Base URL of the mock server (e.g., `http://127.0.0.1:12345`).
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// All requests received by the mock server, in order.
    pub fn received_requests(&self) -> Vec<RecordedRequest> {
        self.inner.requests.lock().unwrap().clone()
    }

    /// Number of queued responses remaining.
    pub fn responses_remaining(&self) -> usize {
        self.inner.responses.lock().unwrap().len()
    }

    /// Explicitly shut down the mock server.
    pub async fn shutdown(mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
            let _ = h.await;
        }
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Builder for MockUpstream.
pub struct MockUpstreamBuilder {
    responses: VecDeque<MockResponse>,
}

impl MockUpstreamBuilder {
    fn new() -> Self {
        Self {
            responses: VecDeque::new(),
        }
    }

    /// Queue a raw MockResponse.
    pub fn response(mut self, resp: MockResponse) -> Self {
        self.responses.push_back(resp);
        self
    }

    /// Queue a JSON response.
    pub fn json(self, status: u16, body: impl Into<String>) -> Self {
        self.response(MockResponse::Json {
            status,
            body: body.into(),
        })
    }

    /// Queue an SSE response with the given chunks.
    /// Each chunk should be a complete SSE frame (e.g., `"data: {...}\n\n"`).
    pub fn sse(self, status: u16, chunks: Vec<impl Into<String>>) -> Self {
        self.response(MockResponse::Sse {
            status,
            chunks: chunks.into_iter().map(Into::into).collect(),
        })
    }

    /// Queue an error response.
    pub fn error(self, status: u16, body: impl Into<String>) -> Self {
        self.response(MockResponse::Error {
            status,
            body: body.into(),
        })
    }

    /// Build and start the mock server.
    pub async fn build(self) -> MockUpstream {
        MockUpstream::start(self.responses).await
    }
}

async fn handle_request(
    State(inner): State<Arc<Inner>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    inner.requests.lock().unwrap().push(RecordedRequest {
        method: method.to_string(),
        path: uri.to_string(),
        headers: headers.clone(),
        body: body.clone(),
    });

    let resp = inner.responses.lock().unwrap().pop_front();
    let Some(resp) = resp else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "MockUpstream: no responses queued",
        )
            .into_response();
    };

    match resp {
        MockResponse::Json { status, body } => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
            [("content-type", "application/json")],
            body,
        )
            .into_response(),
        MockResponse::Sse { status, chunks } => {
            let stream = futures_util::stream::iter(
                chunks.into_iter().map(Ok::<_, std::convert::Infallible>),
            );
            Response::builder()
                .status(status)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .body(Body::from_stream(stream))
                .unwrap()
        }
        MockResponse::Error { status, body } => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body,
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_queued_json() {
        let mock = MockUpstream::builder()
            .json(200, r#"{"ok":true}"#)
            .build()
            .await;

        let resp = reqwest::get(format!("{}/test", mock.url())).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), r#"{"ok":true}"#);

        let reqs = mock.received_requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].path, "/test");
    }

    #[tokio::test]
    async fn mock_returns_fifo_order() {
        let mock = MockUpstream::builder()
            .json(200, "first")
            .json(201, "second")
            .build()
            .await;

        let r1 = reqwest::get(format!("{}/a", mock.url())).await.unwrap();
        let r2 = reqwest::get(format!("{}/b", mock.url())).await.unwrap();

        assert_eq!(r1.status(), 200);
        assert_eq!(r1.text().await.unwrap(), "first");
        assert_eq!(r2.status(), 201);
        assert_eq!(r2.text().await.unwrap(), "second");
    }

    #[tokio::test]
    async fn mock_sse_streams_chunks() {
        let mock = MockUpstream::builder()
            .sse(200, vec!["data: hello\n\n", "data: world\n\n"])
            .build()
            .await;

        let resp = reqwest::get(format!("{}/stream", mock.url()))
            .await
            .unwrap();
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        let body = resp.text().await.unwrap();
        assert!(body.contains("data: hello"));
        assert!(body.contains("data: world"));
    }

    #[tokio::test]
    async fn mock_empty_queue_returns_500() {
        let mock = MockUpstream::builder().build().await;

        let resp = reqwest::get(format!("{}/empty", mock.url())).await.unwrap();
        assert_eq!(resp.status(), 500);
    }
}
