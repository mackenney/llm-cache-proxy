//! SSE detection edge case integration tests.
//!
//! Regression guards for `is_sse_first_chunk` — the function that decides
//! whether a response body should be routed through `SseRestoreStream`.

use crate::common::{MockUpstream, TestHarness};

fn request_body() -> String {
    r#"{"model":"claude-haiku-4-5","max_tokens":10,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#.to_string()
}

async fn post(proxy_url: &str, body: String) -> (u16, bytes::Bytes) {
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

/// First chunk is `data:{` (no space after colon — spaceless SSE format).
///
/// `is_sse_first_chunk` must detect this as SSE and route to `SseRestoreStream`.
/// Regression guard for the spaceless detection fix (commit 0bc6827).
#[tokio::test]
async fn sse_detection_spaceless_data_prefix() {
    let mock = MockUpstream::builder()
        .sse(
            200,
            vec![
                "data:{\"type\":\"message_start\"}\n\n",
                "data:{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                "data:{\"type\":\"message_stop\"}\n\n",
            ],
        )
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (status, bytes) = post(&harness.proxy_url(), request_body()).await;
    assert_eq!(status, 200, "spaceless SSE: must succeed");
    assert!(
        bytes
            .windows(b"message_stop".len())
            .any(|w| w == b"message_stop"),
        "spaceless SSE: all frames must arrive (message_stop present)"
    );
}

/// First chunk is `: OPENROUTER PROCESSING\n\n` (SSE comment line).
///
/// `is_sse_first_chunk` must recognize `: ` prefix as SSE.
/// Regression guard for OpenRouter comment-line handling.
#[tokio::test]
async fn sse_detection_openrouter_comment_prefix() {
    let mock = MockUpstream::builder()
        .sse(
            200,
            vec![
                ": OPENROUTER PROCESSING\n\n",
                ": OPENROUTER PROCESSING\n\n",
                "data: {\"id\":\"1\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ],
        )
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (status, bytes) = post(&harness.proxy_url(), request_body()).await;
    assert_eq!(status, 200, "OR comment: must succeed");
    // Comment lines must pass through unchanged.
    assert!(
        bytes
            .windows(b"OPENROUTER PROCESSING".len())
            .any(|w| w == b"OPENROUTER PROCESSING"),
        "OR comment: comment lines must appear in proxy output"
    );
    // Content must also arrive.
    assert!(
        bytes.windows(b"[DONE]".len()).any(|w| w == b"[DONE]"),
        "OR comment: [DONE] must arrive"
    );
}

/// Non-SSE response (plain JSON) must NOT be routed to SseRestoreStream.
/// The proxy must cache the raw JSON body and return it on HIT.
#[tokio::test]
async fn non_sse_json_response_cached_correctly() {
    let json_body = r#"{"id":"resp_123","object":"response","choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
    let mock = MockUpstream::builder().json(200, json_body).build().await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let (s1, b1) = post(&harness.proxy_url(), request_body()).await;
    assert_eq!(s1, 200);
    // Non-SSE: the response is the raw JSON, not SSE frames.
    assert!(
        b1.windows(b"\"id\":\"resp_123\"".len())
            .any(|w| w == b"\"id\":\"resp_123\""),
        "non-SSE: raw JSON body must pass through"
    );
    harness.wait_for_writes().await;

    // Second request: must be a HIT.
    let client = reqwest::Client::new();
    let r2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    assert_eq!(
        r2.headers().get("x-lcp-cache").unwrap(),
        "HIT",
        "non-SSE JSON must be cached and served on second request"
    );
}
