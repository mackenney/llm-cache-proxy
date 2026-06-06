//! Body limit integration tests.
//!
//! Verify that requests exceeding the configured body limit are rejected
//! with HTTP 413 before reaching the proxy handler.

use crate::common::{MockUpstream, TestHarness};

/// Request body larger than the limit MUST return 413.
#[tokio::test]
async fn test_body_exceeding_limit_returns_413() {
    // Set a small limit: 1024 bytes
    let mock = MockUpstream::builder()
        .json(200, r#"{"content":"should not reach"}"#)
        .build()
        .await;

    let harness = TestHarness::builder()
        .mock(mock)
        .body_limit(1024)
        .build()
        .await;

    let client = reqwest::Client::new();

    // Send a body of 2048 bytes — exceeds the 1024-byte limit
    let large_body = "x".repeat(2048);

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(large_body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "body exceeding limit must return 413 Payload Too Large"
    );
}

/// Request body within the limit MUST pass through to upstream.
#[tokio::test]
async fn test_body_within_limit_passes_through() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"id":"msg_123","content":[{"text":"hello"}],"model":"claude-sonnet-4-20250514","type":"message"}"#)
        .build()
        .await;

    // Set limit: 4096 bytes
    let harness = TestHarness::builder()
        .mock(mock)
        .body_limit(4096)
        .build()
        .await;

    let client = reqwest::Client::new();

    // Send a body of 1024 bytes — well within the 4096-byte limit
    let body = format!(
        r#"{{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{{"role":"user","content":"{}"}}]}}"#,
        "x".repeat(900) // pad to make body ~1KB
    );

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "body within limit must pass through; got {}",
        resp.status()
    );
}
