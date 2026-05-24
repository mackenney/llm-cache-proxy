//! Cache HIT behavior tests — SPEC.md section on proxy cache-hit flow.

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#
}

async fn setup_hit_harness() -> (TestHarness, String) {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
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

    let first_body = resp.text().await.unwrap();
    // Yield to let the proxy's spawned cache-write task complete before tests check HIT.
    harness.wait_for_writes().await;
    (harness, first_body)
}

#[tokio::test]
async fn test_hit_adds_x_lcp_cache_hit_header() {
    let (harness, _) = setup_hit_harness().await;
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
    assert_eq!(cache_header, "HIT");
}

#[tokio::test]
async fn test_hit_adds_x_lcp_key_header() {
    let (harness, _) = setup_hit_harness().await;
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
async fn test_hit_increments_hit_count() {
    let (harness, _) = setup_hit_harness().await;
    let client = reqwest::Client::new();

    let entries_before = harness.cache().list_entries().unwrap();
    assert_eq!(entries_before.len(), 1);
    let hit_count_before = entries_before[0].hit_count;

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries_after = harness.cache().list_entries().unwrap();
    assert_eq!(
        entries_after[0].hit_count,
        hit_count_before + 1,
        "hit_count should increment on HIT"
    );
}

#[tokio::test]
async fn test_hit_increments_stats() {
    let (harness, _) = setup_hit_harness().await;
    let client = reqwest::Client::new();

    let stats_before = harness.cache().stats().unwrap();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let stats_after = harness.cache().stats().unwrap();
    assert!(
        stats_after.hits > stats_before.hits,
        "hits stat should increment"
    );
    assert!(
        stats_after.bytes_served_from_cache > stats_before.bytes_served_from_cache,
        "bytes_served_from_cache should increment"
    );
}

#[tokio::test]
async fn test_hit_replays_stored_chunks() {
    let (harness, first_body) = setup_hit_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();

    let second_body = resp.text().await.unwrap();

    assert_eq!(
        first_body, second_body,
        "HIT response should replay stored chunks exactly"
    );

    assert_eq!(
        harness.mock_requests().len(),
        1,
        "mock should only receive one request"
    );
}
