//! Admin endpoint tests — SPEC contract for /, /stats, /cache admin routes.

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#
}

async fn send_request(client: &reqwest::Client, proxy_url: &str) -> reqwest::Response {
    client
        .post(format!("{proxy_url}/anthropic/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn test_health_returns_status_ok() {
    let mock = MockUpstream::builder().build().await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "GET / must return 200");

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["status"].as_str().unwrap_or(""),
        "ok",
        "health response must have status: ok; got: {body}"
    );
}

#[tokio::test]
async fn test_stats_returns_required_fields() {
    let mock = MockUpstream::builder().build().await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "GET /stats must return 200");

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["hits"].is_number(), "stats must have 'hits' integer");
    assert!(
        body["misses"].is_number(),
        "stats must have 'misses' integer"
    );
    assert!(
        body["bytes_served_from_cache"].is_number(),
        "stats must have 'bytes_served_from_cache' integer"
    );
    assert!(
        body["entries"].is_number(),
        "stats must have 'entries' integer"
    );
    assert!(
        body["by_model"].is_object(),
        "stats must have 'by_model' object"
    );
}

#[tokio::test]
async fn test_stats_after_cache_operations() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response()) // second queued but won't be used (HIT serves from cache)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // First request: MISS
    let resp1 = send_request(&client, &harness.proxy_url()).await;
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let stats_after_miss: serde_json::Value = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        stats_after_miss["misses"].as_u64().unwrap_or(0),
        1,
        "one MISS must increment misses to 1"
    );

    // Second request: HIT (same body)
    let resp2 = send_request(&client, &harness.proxy_url()).await;
    assert_eq!(
        resp2.headers().get("x-lcp-cache").expect("x-lcp-cache"),
        "HIT"
    );
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let stats_after_hit: serde_json::Value = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        stats_after_hit["hits"].as_u64().unwrap_or(0),
        1,
        "one HIT must increment hits to 1"
    );
    assert!(
        stats_after_hit["bytes_served_from_cache"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "bytes_served_from_cache must be > 0 after a HIT"
    );
}

#[tokio::test]
async fn test_delete_stats_resets_counters_not_entries() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // Create a cache entry.
    let resp = send_request(&client, &harness.proxy_url()).await;
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Verify 1 miss and 1 entry.
    let stats_before: serde_json::Value = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats_before["misses"].as_u64().unwrap_or(0), 1);
    assert_eq!(stats_before["entries"].as_u64().unwrap_or(0), 1);

    // Reset stats.
    let del_resp = client
        .delete(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), 200, "DELETE /stats must return 200");
    let del_body: serde_json::Value = del_resp.json().await.unwrap();
    assert_eq!(
        del_body["cleared"].as_bool(),
        Some(true),
        "DELETE /stats must return {{\"cleared\": true}}; got: {del_body}"
    );

    // Counters must be zero; entries must remain.
    let stats_after: serde_json::Value = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        stats_after["hits"].as_u64().unwrap_or(999),
        0,
        "hits must be 0 after DELETE /stats"
    );
    assert_eq!(
        stats_after["misses"].as_u64().unwrap_or(999),
        0,
        "misses must be 0 after DELETE /stats"
    );
    assert_eq!(
        stats_after["bytes_served_from_cache"]
            .as_u64()
            .unwrap_or(999),
        0,
        "bytes_served_from_cache must be 0 after DELETE /stats"
    );
    assert_eq!(
        stats_after["entries"].as_u64().unwrap_or(0),
        1,
        "entries count must remain 1 after DELETE /stats (entries are not deleted)"
    );

    // Direct cache check: entry still exists.
    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1, "cache entry must survive DELETE /stats");
}

#[tokio::test]
async fn test_delete_cache_clears_entries_not_stats() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // Create a cache entry.
    let resp = send_request(&client, &harness.proxy_url()).await;
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Note misses before clearing cache.
    let stats_before: serde_json::Value = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats_before["misses"].as_u64().unwrap_or(0), 1);

    // Clear cache entries.
    let del_resp = client
        .delete(format!("{}/cache", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(del_resp.status(), 200, "DELETE /cache must return 200");
    let del_cache_body: serde_json::Value = del_resp.json().await.unwrap();
    assert!(
        del_cache_body["cleared_entries"].is_number(),
        "DELETE /cache must return {{\"cleared_entries\": N}}; got: {del_cache_body}"
    );

    // Entries must be gone; stats counters must be unchanged.
    let stats_after: serde_json::Value = client
        .get(format!("{}/stats", harness.proxy_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        stats_after["misses"].as_u64().unwrap_or(0),
        1,
        "misses stat must remain 1 after DELETE /cache"
    );
    assert_eq!(
        stats_after["entries"].as_u64().unwrap_or(999),
        0,
        "entries must be 0 after DELETE /cache"
    );

    // Direct cache check: no entries.
    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 0, "cache must be empty after DELETE /cache");
}

#[tokio::test]
async fn test_get_cache_entry_returns_full_exchange() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = send_request(&client, &harness.proxy_url()).await;
    // x-lcp-key header is only 12 chars; get the full key from the cache.
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let cache_entries = harness.cache().list_entries().unwrap();
    assert_eq!(
        cache_entries.len(),
        1,
        "one entry must be in cache after MISS"
    );
    let key = &cache_entries[0].key;

    let entry_resp = client
        .get(format!("{}/cache/{}", harness.proxy_url(), key))
        .send()
        .await
        .unwrap();
    assert_eq!(entry_resp.status(), 200, "GET /cache/<key> must return 200");

    let body: serde_json::Value = entry_resp.json().await.unwrap();
    assert_eq!(
        body["key"].as_str().unwrap_or(""),
        key,
        "entry 'key' must match requested key"
    );
    assert!(
        body["created_at"].is_string(),
        "entry must have 'created_at'"
    );
    assert!(body["provider"].is_string(), "entry must have 'provider'");
    assert!(body["status"].is_number(), "entry must have 'status'");
    assert!(body["hit_count"].is_number(), "entry must have 'hit_count'");
    assert!(
        body["request"].is_object() || body["request"].is_string(),
        "entry must have 'request'"
    );
    assert!(body["chunks"].is_array(), "entry must have 'chunks' array");
}

#[tokio::test]
async fn test_get_cache_entry_unknown_key_returns_404() {
    let mock = MockUpstream::builder().build().await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/cache/nonexistent123abc", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        404,
        "GET /cache/<nonexistent-key> must return 404"
    );
}
