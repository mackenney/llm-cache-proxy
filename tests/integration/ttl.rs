//! TTL expiry integration tests.
//!
//! These tests sleep for 1-2 seconds to let entries expire, so they live in
//! the integration tier where timing-sensitive tests are acceptable.

use std::time::Duration;

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"ttl-test"}]}"#
}

#[tokio::test]
async fn test_ttl_nonzero_entry_expires_after_ttl() {
    // TTL = 1 second.
    let mock = MockUpstream::builder()
        .sse(200, sse_response()) // first request: MISS
        .sse(200, sse_response()) // second request: MISS (expired)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).ttl(1).build().await;
    let client = reqwest::Client::new();

    // First request: MISS; entry stored with 1-second TTL.
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp1
            .headers()
            .get("x-lcp-cache")
            .expect("x-lcp-cache")
            .as_bytes(),
        b"MISS",
        "first request must be a MISS"
    );
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Verify entry exists in cache.
    let entries_before = harness.cache().list_entries().unwrap();
    assert_eq!(entries_before.len(), 1, "one entry must exist after MISS");

    // Wait for TTL to expire (2 seconds > 1 second TTL).
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Second request: same body, but entry is expired — must be MISS.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let cache_header2 = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert_eq!(
        cache_header2, "MISS",
        "expired entry must produce a MISS; got: {cache_header2}"
    );

    // Stats: only the initial MISS increments the counter.
    // The expired-row path returns Ok(None) without calling the misses increment
    // (it returns None before the counter logic runs).
    let stats = harness.cache().stats().unwrap();
    assert_eq!(
        stats.misses, 1,
        "only initial MISS increments counter; expired-row path does not; got: {}",
        stats.misses
    );
}

#[tokio::test]
async fn test_ttl_zero_entry_never_expires() {
    // TTL = 0 means never expire.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        // No second response queued — if the second request hits upstream, it gets a 500.
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).ttl(0).build().await;
    let client = reqwest::Client::new();

    // First request: MISS; entry stored with TTL=0 (never expires).
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Wait 2 seconds — entry must NOT expire with TTL=0.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Second request: same body — must still be a HIT.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let cache_header2 = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();

    assert_eq!(
        cache_header2, "HIT",
        "TTL=0 entry must not expire; expected HIT after 2s, got: {cache_header2}"
    );

    // Only 1 upstream call (the initial MISS).
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "only one upstream call expected — HIT must be served from cache"
    );
}
