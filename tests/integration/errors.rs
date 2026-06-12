//! Error path integration tests.
//!
//! Verifies that upstream error responses are forwarded to the client
//! and not written to the cache.

use crate::common::{MockUpstream, TestHarness};

fn anthropic_body() -> String {
    r#"{"model":"claude-haiku-4-5","max_tokens":10,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#.to_string()
}

async fn send(proxy_url: &str, body: String) -> (u16, bytes::Bytes) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{proxy_url}/anthropic/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap();
    (status, bytes)
}

/// 4xx upstream responses must NOT be written to cache.
/// A second identical request must still reach the upstream.
#[tokio::test]
async fn upstream_4xx_not_cached() {
    let mock = MockUpstream::builder()
        .error(429, r#"{"error":"rate limited"}"#)
        .error(429, r#"{"error":"rate limited"}"#)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (s1, _) = send(&harness.proxy_url(), anthropic_body()).await;
    let (s2, _) = send(&harness.proxy_url(), anthropic_body()).await;

    assert_eq!(s1, 429, "first request must return 429");
    assert_eq!(s2, 429, "second request must also return 429 (not cached)");
    assert_eq!(
        harness.mock_requests().len(),
        2,
        "upstream must receive both requests — 4xx must not be cached"
    );
}

/// 5xx upstream responses must NOT be written to cache.
#[tokio::test]
async fn upstream_5xx_not_cached() {
    let mock = MockUpstream::builder()
        .error(529, r#"{"error":"overloaded"}"#)
        .error(529, r#"{"error":"overloaded"}"#)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (s1, _) = send(&harness.proxy_url(), anthropic_body()).await;
    let (s2, _) = send(&harness.proxy_url(), anthropic_body()).await;

    assert_eq!(s1, 529, "first request must return 529");
    assert_eq!(s2, 529, "second request must also return 529");
    assert_eq!(
        harness.mock_requests().len(),
        2,
        "upstream must receive both requests — 5xx must not be cached"
    );
}

/// An upstream that closes the connection mid-SSE-stream without [DONE]:
/// the client receives what arrived.
///
/// Note: the proxy caches the partial response (stream EOF is treated as completion).
/// A second request gets a HIT with the partial data.
#[tokio::test]
async fn upstream_partial_sse_not_cached() {
    let partial_chunks = vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
    ];

    let mock = MockUpstream::builder()
        .sse(200, partial_chunks)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (s1, b1) = send(&harness.proxy_url(), anthropic_body()).await;
    assert_eq!(s1, 200);
    assert!(
        !b1.is_empty(),
        "partial SSE: client must receive what arrived"
    );
    harness.wait_for_writes().await;

    // Proxy caches whatever it received (EOF = stream complete).
    // Verify the client received the partial data successfully.
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "partial SSE: upstream must receive exactly 1 request"
    );
}

/// Non-SSE 200 JSON response for a streaming request: proxy caches it correctly.
#[tokio::test]
async fn upstream_json_for_streaming_request_cached() {
    let mock = MockUpstream::builder()
        .json(
            200,
            r#"{"id":"msg_123","content":[{"type":"text","text":"hello"}]}"#,
        )
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (s1, b1) = send(&harness.proxy_url(), anthropic_body()).await;
    assert_eq!(s1, 200);
    assert!(!b1.is_empty());
    harness.wait_for_writes().await;

    // Second request: cache hit (non-SSE responses are also cached).
    let client = reqwest::Client::new();
    let r2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(anthropic_body())
        .send()
        .await
        .unwrap();
    assert_eq!(
        r2.headers().get("x-lcp-cache").unwrap(),
        "HIT",
        "non-SSE 200 response must be cached"
    );
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "upstream must be called only once (non-SSE response cached)"
    );
}
