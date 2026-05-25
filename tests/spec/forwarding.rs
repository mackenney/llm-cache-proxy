//! Forwarding tests — SPEC contract: proxy strips hop-by-hop and client headers before upstream.

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

#[tokio::test]
async fn test_accept_encoding_stripped_before_forwarding() {
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
        .header("accept-encoding", "gzip, br")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();

    let reqs = harness.mock_requests();
    assert_eq!(reqs.len(), 1, "expected exactly one upstream request");
    assert!(
        reqs[0].headers.get("accept-encoding").is_none(),
        "accept-encoding must be stripped before forwarding to upstream; got: {:?}",
        reqs[0].headers.get("accept-encoding")
    );
}

#[tokio::test]
async fn test_host_header_stripped_before_forwarding() {
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
        .header("host", "api.anthropic.com")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();

    let reqs = harness.mock_requests();
    // The Host header forwarded to mock must be the mock's own address, not the client-supplied value.
    // Alternatively, the proxy may strip it entirely and let hyper set it from the upstream URL.
    // Either way, the client-supplied value "api.anthropic.com" must not reach upstream.
    let forwarded_host = reqs[0]
        .headers
        .get("host")
        .map(|v| v.to_str().unwrap_or("").to_string());
    assert!(
        forwarded_host
            .as_deref()
            .map(|h| !h.contains("api.anthropic.com"))
            .unwrap_or(true),
        "client-supplied Host header must not be forwarded to upstream; got: {forwarded_host:?}"
    );
}

#[tokio::test]
async fn test_connection_header_stripped_before_forwarding() {
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
        .header("connection", "keep-alive")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();

    let reqs = harness.mock_requests();
    assert!(
        reqs[0].headers.get("connection").is_none(),
        "connection header must be stripped before forwarding to upstream; got: {:?}",
        reqs[0].headers.get("connection")
    );
}
