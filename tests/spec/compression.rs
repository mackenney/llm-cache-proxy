//! Compression tests — SPEC contract: compressed request body is decompressed before cache key hashing.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::common::{MockUpstream, TestHarness};

fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

#[tokio::test]
async fn test_compressed_request_body_decompressed_before_hashing() {
    // Two SSE responses queued — if the second request is a cache HIT, the second
    // queued response is never consumed. If it is a MISS, both are consumed.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let plain_body = r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"compress-test"}]}"#;
    let compressed_body = gzip_compress(plain_body.as_bytes());

    // First request: gzip-compressed body.
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("content-encoding", "gzip")
        .body(compressed_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);
    let cache_header1 = resp1
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_header1, "MISS", "first request must be MISS");
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Second request: identical plain-text body — must be a HIT.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(plain_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
    let cache_header2 = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        cache_header2, "HIT",
        "plain-body request identical to earlier compressed request must be a cache HIT — \
         compressed and plain bodies must hash to the same key"
    );

    // Upstream must have been called exactly once (the compressed MISS);
    // the HIT was served from cache.
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "upstream must be called exactly once — HIT must be served from cache"
    );
}

#[tokio::test]
async fn test_content_encoding_stripped_before_forwarding() {
    // Even if the downstream sends Content-Encoding, the upstream must never
    // receive it: lcp decompresses the body and owns the strip obligation.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let plain_body = concat!(
        r#"{"model":"claude-sonnet-4-20250514","#,
        r#""max_tokens":10,"messages":[]}"#,
    );
    let compressed_body = gzip_compress(plain_body.as_bytes());

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("content-encoding", "gzip")
        .body(compressed_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();

    let reqs = harness.mock_requests();
    assert_eq!(reqs.len(), 1);
    let forwarded_headers = &reqs[0].headers;
    assert!(
        !forwarded_headers.contains_key("content-encoding"),
        "content-encoding must not be forwarded to the upstream after decompression"
    );
}
