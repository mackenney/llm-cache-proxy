//! Routing tests — SPEC contract: unknown provider prefix → 404.

use crate::common::{MockUpstream, TestHarness};

#[tokio::test]
async fn test_unknown_provider_returns_404() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"ok":true}"#)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/badprovider/v1/messages", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(r#"{"model":"test","messages":[]}"#)
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        404,
        "unknown provider prefix must return 404"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.to_lowercase().contains("unknown")
            || body.to_lowercase().contains("provider")
            || body.to_lowercase().contains("not found"),
        "404 body must mention unknown/provider/not found; got: {body}"
    );
}

#[tokio::test]
async fn test_valid_provider_does_not_return_404() {
    let mock = MockUpstream::builder()
        .sse(
            200,
            vec![
                "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        )
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();

    assert_ne!(
        resp.status(),
        404,
        "valid anthropic prefix must not return 404"
    );
}
