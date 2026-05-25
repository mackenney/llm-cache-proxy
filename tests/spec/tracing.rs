//! Tracing tests — SPEC contract for /trace/<id> endpoint and trace aggregation.

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

async fn send_traced_request(
    client: &reqwest::Client,
    proxy_url: &str,
    trace_id: &str,
    body: &str,
) -> reqwest::Response {
    client
        .post(format!("{proxy_url}/anthropic/v1/messages"))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("x-lcp-trace", trace_id)
        .body(body.to_string())
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn test_trace_endpoint_returns_metadata() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = send_traced_request(
        &client,
        &harness.proxy_url(),
        "my-trace-001",
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let trace_resp = client
        .get(format!("{}/trace/my-trace-001", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(trace_resp.status(), 200, "GET /trace/<id> must return 200");

    let body: serde_json::Value = trace_resp.json().await.unwrap();
    assert_eq!(
        body["trace_id"].as_str().unwrap_or(""),
        "my-trace-001",
        "trace_id field must match the requested trace ID"
    );
    let entries = body["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(entries.len(), 1, "one request must produce one trace entry");

    let entry = &entries[0];
    assert!(entry["key"].is_string(), "entry must have a 'key' field");
    assert!(
        entry["created_at"].is_string(),
        "entry must have a 'created_at' field"
    );
    assert!(
        entry["status"].is_number(),
        "entry must have a 'status' field"
    );
    assert!(
        entry["hit_count"].is_number(),
        "entry must have a 'hit_count' field"
    );
}

#[tokio::test]
async fn test_trace_endpoint_full_includes_request_and_chunks() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = send_traced_request(
        &client,
        &harness.proxy_url(),
        "my-trace-full",
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let trace_resp = client
        .get(format!(
            "{}/trace/my-trace-full?full=true",
            harness.proxy_url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(trace_resp.status(), 200);

    let body: serde_json::Value = trace_resp.json().await.unwrap();
    let entries = body["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(entries.len(), 1);

    let entry = &entries[0];
    assert!(
        entry["request"].is_object() || entry["request"].is_string(),
        "full=true entry must include 'request' field; got: {entry}"
    );
    assert!(
        entry["chunks"].is_array(),
        "full=true entry must include 'chunks' array; got: {entry}"
    );
}

#[tokio::test]
async fn test_trace_unknown_id_returns_empty_entries() {
    let mock = MockUpstream::builder().build().await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let trace_resp = client
        .get(format!(
            "{}/trace/nonexistent-trace-xyz",
            harness.proxy_url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(
        trace_resp.status(),
        200,
        "unknown trace ID must return 200 with empty entries"
    );

    let body: serde_json::Value = trace_resp.json().await.unwrap();
    let entries = body["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(
        entries.len(),
        0,
        "unknown trace ID must return empty entries array"
    );
}

#[tokio::test]
async fn test_multiple_requests_same_trace_aggregates_entries() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // Two different bodies, same trace ID — each produces a distinct cache entry.
    let resp1 = send_traced_request(
        &client,
        &harness.proxy_url(),
        "session-abc",
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"first"}]}"#,
    )
    .await;
    let _ = resp1.bytes().await.unwrap();

    let resp2 = send_traced_request(
        &client,
        &harness.proxy_url(),
        "session-abc",
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"second"}]}"#,
    )
    .await;
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let trace_resp = client
        .get(format!("{}/trace/session-abc", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = trace_resp.json().await.unwrap();
    let entries = body["entries"]
        .as_array()
        .expect("entries must be an array");
    assert_eq!(
        entries.len(),
        2,
        "two requests with same trace ID must produce two trace entries"
    );

    let key0 = entries[0]["key"].as_str().unwrap_or("");
    let key1 = entries[1]["key"].as_str().unwrap_or("");
    assert_ne!(
        key0, key1,
        "distinct requests must produce distinct cache keys in the trace"
    );
}

#[tokio::test]
async fn test_same_cache_key_multiple_traces_many_to_many() {
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let body = r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"same"}]}"#;

    // First request: trace-1, cache MISS.
    let resp1 = send_traced_request(&client, &harness.proxy_url(), "trace-alpha", body).await;
    let _ = resp1.bytes().await.unwrap();

    // Second request: trace-2, same body → cache HIT.
    let resp2 = send_traced_request(&client, &harness.proxy_url(), "trace-beta", body).await;
    let cache_header = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present");
    assert_eq!(
        cache_header, "HIT",
        "second identical request must be a cache HIT"
    );
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Get the full cache key from the cache entries (x-lcp-key header is only 12 chars).
    let cache_entries = harness.cache().list_entries().unwrap();
    assert_eq!(cache_entries.len(), 1, "only one distinct entry must exist");
    let full_key = cache_entries[0].key.clone();

    // Both traces must include the same cache key.
    let check_trace = |trace_id: &str| {
        let trace_id = trace_id.to_string();
        let full_key = full_key.clone();
        let proxy_url = harness.proxy_url();
        let client = client.clone();
        async move {
            let r = client
                .get(format!("{proxy_url}/trace/{trace_id}"))
                .send()
                .await
                .unwrap();
            let b: serde_json::Value = r.json().await.unwrap();
            let entries = b["entries"].as_array().expect("entries array");
            assert_eq!(entries.len(), 1, "trace {trace_id} must have 1 entry");
            let key = entries[0]["key"].as_str().unwrap_or("");
            assert_eq!(
                key, full_key,
                "trace {trace_id} must reference the same cache key"
            );
        }
    };

    check_trace("trace-alpha").await;
    check_trace("trace-beta").await;
}
