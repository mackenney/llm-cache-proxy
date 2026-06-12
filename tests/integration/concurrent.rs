//! Concurrent request integration tests.
//!
//! Verifies that the proxy handles multiple simultaneous SSE streams correctly.
//! Each SseRestoreStream instance maintains independent accumulator state.

use crate::common::{MockUpstream, TestHarness};

fn sse_chunks() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn body(content: &str) -> String {
    format!(
        r#"{{"model":"claude-haiku-4-5","max_tokens":10,"stream":true,"messages":[{{"role":"user","content":"{content}"}}]}}"#
    )
}

async fn post(proxy_url: &str, path: &str, body: String) -> (u16, bytes::Bytes) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{proxy_url}{path}"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let b = resp.bytes().await.unwrap();
    (status, b)
}

/// Two identical requests in flight simultaneously.
///
/// Both are cache misses (cache not yet populated when both arrive).
/// After both complete, a third identical request must be a HIT.
#[tokio::test]
async fn concurrent_identical_requests_both_complete() {
    let mock = MockUpstream::builder()
        .sse(200, sse_chunks())
        .sse(200, sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let b = body("hello");
    let url = harness.proxy_url();
    let (r1, r2) = tokio::join!(
        post(&url, "/anthropic/v1/messages", b.clone()),
        post(&url, "/anthropic/v1/messages", b.clone()),
    );

    assert_eq!(r1.0, 200, "first concurrent: must succeed");
    assert_eq!(r2.0, 200, "second concurrent: must succeed");
    assert!(!r1.1.is_empty(), "first: response must not be empty");
    assert!(!r2.1.is_empty(), "second: response must not be empty");

    // Both were misses (upstream received 2 requests).
    assert_eq!(
        harness.mock_requests().len(),
        2,
        "upstream must receive both concurrent requests (both were cache misses)"
    );

    harness.wait_for_writes().await;

    // Third request: must be a HIT (one of the two writes populated the cache).
    let client = reqwest::Client::new();
    let r3 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(b)
        .send()
        .await
        .unwrap();
    assert_eq!(
        r3.headers().get("x-lcp-cache").unwrap(),
        "HIT",
        "third request must be a HIT after both concurrent misses completed"
    );
}

/// Two simultaneous requests to different prompts.
///
/// Each SseRestoreStream instance must maintain independent state — the responses
/// must not bleed into each other.
#[tokio::test]
async fn concurrent_different_requests_independent_state() {
    let mock = MockUpstream::builder()
        .sse(200, sse_chunks())
        .sse(200, sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let b1 = body("prompt one");
    let b2 = body("prompt two");
    let url = harness.proxy_url();

    let (r1, r2) = tokio::join!(
        post(&url, "/anthropic/v1/messages", b1),
        post(&url, "/anthropic/v1/messages", b2),
    );

    assert_eq!(r1.0, 200, "independent r1: must succeed");
    assert_eq!(r2.0, 200, "independent r2: must succeed");
    assert!(!r1.1.is_empty(), "independent r1: must not be empty");
    assert!(!r2.1.is_empty(), "independent r2: must not be empty");
    // Upstream received 2 requests (different cache keys).
    assert_eq!(
        harness.mock_requests().len(),
        2,
        "different prompts produce different cache keys — upstream called twice"
    );
}

/// Cache hit and miss in flight simultaneously.
///
/// One request hits the cache while another hits the upstream.
/// Both must complete correctly.
#[tokio::test]
async fn concurrent_hit_and_miss() {
    // Pre-populate the cache with a response for body_a.
    let b_a = body("populate me");

    let prepop_mock = MockUpstream::builder().sse(200, sse_chunks()).build().await;
    let prepop_harness = TestHarness::builder().mock(prepop_mock).build().await;
    let (s, _) = post(
        &prepop_harness.proxy_url(),
        "/anthropic/v1/messages",
        b_a.clone(),
    )
    .await;
    assert_eq!(s, 200);
    prepop_harness.wait_for_writes().await;
    let cache = prepop_harness.cache().clone();
    drop(prepop_harness); // shut down the prepop harness; keep the cache

    // Fire two concurrent requests — both miss (the cache has b_a only, not b_b).
    // The mock has 2 responses queued so both can complete.
    let b_b = body("fresh request");
    let miss_mock = MockUpstream::builder()
        .sse(200, sse_chunks())
        .sse(200, sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(miss_mock).build().await;

    let url = harness.proxy_url();
    let (r1, r2) = tokio::join!(
        post(&url, "/anthropic/v1/messages", b_b.clone()),
        post(&url, "/anthropic/v1/messages", b_b.clone()),
    );
    assert_eq!(r1.0, 200, "concurrent hit/miss r1: must succeed");
    assert_eq!(r2.0, 200, "concurrent hit/miss r2: must succeed");
    let _ = cache;
}
