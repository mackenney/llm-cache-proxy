//! TestHarness — wires MockUpstream, proxy server, and cache for integration testing.

use std::net::SocketAddr;
use std::sync::Arc;

use lcp_core::Cache;
use lcp_server::proxy::AppState;
use lcp_server::router::build_router;
use lcp_server::server::ServerConfig;
use tokio::task::JoinHandle;

use super::mock_upstream::MockUpstream;

/// Test harness combining MockUpstream, lcp proxy, and in-memory cache.
///
/// # Example
/// ```ignore
/// let harness = TestHarness::builder()
///     .mock(MockUpstream::builder().json(200, "{}").build().await)
///     .build()
///     .await;
///
/// let resp = reqwest::get(format!("{}/anthropic/v1/messages", harness.proxy_url()))
///     .await.unwrap();
/// ```
pub struct TestHarness {
    pub mock: MockUpstream,
    cache: Cache,
    proxy_addr: SocketAddr,
    proxy_handle: Option<JoinHandle<()>>,
    app_state: AppState,
}

impl TestHarness {
    /// Start building a test harness.
    pub fn builder() -> TestHarnessBuilder {
        TestHarnessBuilder::new()
    }

    /// URL of the proxy server (e.g., `http://127.0.0.1:54321`).
    pub fn proxy_url(&self) -> String {
        format!("http://{}", self.proxy_addr)
    }

    /// Direct access to the cache for assertions.
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// URL of the mock upstream server.
    pub fn mock_url(&self) -> String {
        self.mock.url()
    }

    /// Requests received by the mock upstream.
    pub fn mock_requests(&self) -> Vec<super::mock_upstream::RecordedRequest> {
        self.mock.received_requests()
    }

    /// Explicitly shut down both mock and proxy servers.
    pub async fn shutdown(mut self) {
        if let Some(h) = self.proxy_handle.take() {
            h.abort();
            let _ = h.await;
        }
        // self.mock is dropped here, triggering MockUpstream::drop()
    }

    /// Wait for all background cache writes to complete.
    pub async fn wait_for_writes(&self) {
        self.app_state.wait_for_pending_writes().await;
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        if let Some(h) = self.proxy_handle.take() {
            h.abort();
        }
    }
}

/// Builder for TestHarness.
pub struct TestHarnessBuilder {
    mock: Option<MockUpstream>,
    timeout_seconds: u64,
}

impl TestHarnessBuilder {
    fn new() -> Self {
        Self {
            mock: None,
            timeout_seconds: 30,
        }
    }

    /// Set the mock upstream server (required).
    pub fn mock(mut self, mock: MockUpstream) -> Self {
        self.mock = Some(mock);
        self
    }

    /// Set proxy timeout in seconds (default: 30).
    pub fn timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = seconds;
        self
    }

    /// Build and start the test harness.
    ///
    /// # Panics
    /// Panics if `mock()` was not called.
    pub async fn build(self) -> TestHarness {
        let mock = self.mock.expect("TestHarness requires a MockUpstream");
        let mock_url = mock.url();

        let cache = Cache::open(&":memory:".into(), 0).expect("open in-memory cache");

        let config = ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            cache: cache.clone(),
            timeout_seconds: self.timeout_seconds,
            anthropic_upstream: Some(mock_url.clone()),
            openai_upstream: Some(mock_url.clone()),
            openrouter_upstream: Some(mock_url.clone()),
            gemini_upstream: Some(mock_url.clone()),
            stream_channel_capacity: 32,
        };

        let client = Arc::new(
            reqwest::Client::builder()
                .no_gzip()
                .no_deflate()
                .no_brotli()
                .build()
                .expect("build reqwest client"),
        );

        let app_state = AppState {
            config: Arc::new(config),
            client,
            background_writes: Arc::new(tokio::sync::Mutex::new(tokio::task::JoinSet::new())),
        };

        let app = build_router(app_state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let proxy_addr = listener.local_addr().expect("get proxy addr");

        let proxy_handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        TestHarness {
            mock,
            cache,
            proxy_addr,
            proxy_handle: Some(proxy_handle),
            app_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::MockUpstream;

    #[tokio::test]
    async fn harness_proxy_responds() {
        let mock = MockUpstream::builder()
            .json(200, r#"{"status":"ok"}"#)
            .build()
            .await;

        let harness = TestHarness::builder().mock(mock).build().await;

        let resp = reqwest::get(format!("{}/", harness.proxy_url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn harness_routes_to_mock() {
        let mock = MockUpstream::builder()
            .sse(
                200,
                vec![
                    "event: message_start\ndata: {}\n\n",
                    "event: message_stop\ndata: {}\n\n",
                ],
            )
            .build()
            .await;

        let harness = TestHarness::builder().mock(mock).build().await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[]}"#)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);

        let reqs = harness.mock_requests();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].path().contains("v1/messages"));
    }

    #[tokio::test]
    async fn harness_cache_accessible() {
        let mock = MockUpstream::builder()
            .json(200, r#"{"ok":true}"#)
            .build()
            .await;

        let harness = TestHarness::builder().mock(mock).build().await;

        let stats = harness.cache().stats().unwrap();
        assert_eq!(stats.entries, 0);
    }
}
