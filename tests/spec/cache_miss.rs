//! Cache MISS behavior tests — SPEC.md section on proxy cache-miss flow.

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hi\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":100,"messages":[{"role":"user","content":"hello"}]}"#
}

async fn setup_miss_harness() -> TestHarness {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    TestHarness::builder().mock(mock).build().await
}

#[tokio::test]
async fn test_miss_adds_x_lcp_cache_miss_header() {
    let harness = setup_miss_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let cache_header = resp
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache header");
    assert_eq!(cache_header, "MISS");
}

#[tokio::test]
async fn test_miss_adds_x_lcp_key_header() {
    let harness = setup_miss_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();

    let key_header = resp.headers().get("x-lcp-key").expect("x-lcp-key header");
    let key = key_header.to_str().unwrap();
    assert_eq!(key.len(), 12);
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn test_miss_stores_2xx_response() {
    let harness = setup_miss_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();

    let _ = resp.bytes().await.unwrap();

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1, "expected 1 cache entry after MISS");
    assert_eq!(entries[0].status, 200);
}

#[tokio::test]
async fn test_miss_does_not_store_non_2xx() {
    let mock = MockUpstream::builder()
        .error(400, r#"{"error":"bad request"}"#)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let _ = resp.bytes().await.unwrap();

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 0, "non-2xx should not be cached");
}

#[tokio::test]
async fn test_miss_increments_misses_stat() {
    let harness = setup_miss_harness().await;
    let client = reqwest::Client::new();

    let stats_before = harness.cache().stats().unwrap();
    assert_eq!(stats_before.misses, 0);

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();

    let stats_after = harness.cache().stats().unwrap();
    assert_eq!(stats_after.misses, 1, "misses stat should increment");
}
