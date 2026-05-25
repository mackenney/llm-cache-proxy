//! Timeout and unreachable upstream integration tests.
//!
//! These tests involve real network timing, so they live in the integration
//! tier (not spec tier) where slow tests are acceptable.

use std::time::Duration;

use crate::common::{MockUpstream, TestHarness};

#[tokio::test]
async fn test_upstream_timeout_returns_gateway_error() {
    // MockUpstream queued with Hang — it sleeps 1 hour before replying.
    let mock = MockUpstream::builder().hang().build().await;
    // Proxy timeout: 1 second. The test wrapper allows up to 5 seconds total.
    let harness = TestHarness::builder().mock(mock).timeout(1).build().await;
    let client = reqwest::Client::new();

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client
            .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#)
            .send(),
    )
    .await;

    let resp = result
        .expect("test timed out — proxy did not respond within 5s (expected ~1s)")
        .expect("reqwest error sending request");

    let status = resp.status().as_u16();
    assert!(
        status == 502 || status == 504,
        "proxy must return 502 or 504 when upstream hangs past timeout; got: {status}"
    );
}

#[tokio::test]
async fn test_upstream_unreachable_returns_502() {
    // Build a mock, capture its URL, immediately shut it down.
    // That address now has no listener — connection will be refused.
    let dead_mock = MockUpstream::builder().json(200, "{}").build().await;
    let dead_url = dead_mock.url();
    dead_mock.shutdown().await;

    // Build a separate live mock for the harness (builder requires one),
    // but override the upstream URL to the dead address so the proxy
    // talks to the closed port instead of the live mock.
    let live_mock = MockUpstream::builder().build().await;
    let harness = TestHarness::builder()
        .mock(live_mock)
        .upstream_url(dead_url)
        .build()
        .await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        502,
        "proxy must return 502 when upstream is unreachable (connection refused)"
    );
}
