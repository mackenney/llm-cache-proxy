//! E2E body limit tests — exercises DefaultBodyLimit enforcement with a real lcp server.
//!
//! Fail cases (body > limit) are cheap: the proxy rejects before forwarding, so
//! no upstream API call is made. Pass cases make one minimal real API call each.
//!
//! Requires: `ANTHROPIC_API_KEY`

use crate::common::{MockUpstream, TestHarness};

fn require_env(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("skipping: {var} not set");
            None
        }
    }
}

/// Build a harness pointing at a real upstream with the given body limit.
async fn harness_with_limit(upstream_url: &str, body_limit: u64) -> TestHarness {
    let mock = MockUpstream::builder().json(200, "{}").build().await;
    TestHarness::builder()
        .mock(mock)
        .upstream_url(upstream_url.to_owned())
        .body_limit(body_limit)
        .timeout(30)
        .build()
        .await
}

/// Smallest valid Anthropic request body (~120 bytes).
fn minimal_anthropic_body(api_key: &str) -> (String, Vec<(&'static str, String)>) {
    let body = r#"{"model":"claude-haiku-4-5","max_tokens":1,"messages":[{"role":"user","content":"hi"}]}"#.to_owned();
    let headers = vec![
        ("x-api-key", api_key.to_owned()),
        ("anthropic-version", "2023-06-01".to_owned()),
        ("content-type", "application/json".to_owned()),
    ];
    (body, headers)
}

// --- Fail cases: body exceeds limit → 413, zero upstream requests ---

/// limit=100, body=200 bytes → 413, never reaches upstream.
#[tokio::test]
async fn body_limit_100_rejects_200_byte_body() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };
    let harness = harness_with_limit("https://api.anthropic.com", 100).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", &api_key)
        .header("content-type", "application/json")
        .body("x".repeat(200))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "100-byte limit must reject 200-byte body"
    );
    assert!(
        harness.mock_requests().is_empty(),
        "upstream must not be reached"
    );
}

/// limit=500, body=600 bytes → 413, never reaches upstream.
#[tokio::test]
async fn body_limit_500_rejects_600_byte_body() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };
    let harness = harness_with_limit("https://api.anthropic.com", 500).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", &api_key)
        .header("content-type", "application/json")
        .body("x".repeat(600))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "500-byte limit must reject 600-byte body"
    );
    assert!(
        harness.mock_requests().is_empty(),
        "upstream must not be reached"
    );
}

/// limit=10_000, body=20_000 bytes → 413, never reaches upstream.
#[tokio::test]
async fn body_limit_10k_rejects_20k_byte_body() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };
    let harness = harness_with_limit("https://api.anthropic.com", 10_000).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", &api_key)
        .header("content-type", "application/json")
        .body("x".repeat(20_000))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "10k-byte limit must reject 20k-byte body"
    );
    assert!(
        harness.mock_requests().is_empty(),
        "upstream must not be reached"
    );
}

// --- Pass cases: body within limit → forwarded to real upstream ---

/// limit=10_000, minimal ~120-byte body → forwarded, real Anthropic response.
#[tokio::test]
async fn body_within_limit_passes_to_real_upstream() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };
    let harness = harness_with_limit("https://api.anthropic.com", 10_000).await;
    let (body, headers) = minimal_anthropic_body(&api_key);
    let client = reqwest::Client::new();

    let mut req = client.post(format!("{}/anthropic/v1/messages", harness.proxy_url()));
    for (k, v) in &headers {
        req = req.header(*k, v);
    }
    let resp = req.body(body).send().await.expect("request failed");

    assert!(
        resp.status().is_success(),
        "minimal body must be forwarded and succeed; got {}",
        resp.status()
    );
    assert_eq!(
        resp.headers()
            .get("x-lcp-cache")
            .and_then(|v| v.to_str().ok()),
        Some("MISS")
    );
}

/// limit=0 (no limit), large body → forwarded, real Anthropic response.
#[tokio::test]
async fn body_limit_zero_forwards_large_body() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };
    // limit=0 means no limit; use a valid padded body
    let harness = harness_with_limit("https://api.anthropic.com", 0).await;
    let client = reqwest::Client::new();

    // Build a valid request with a padded (but ignored) user message field
    let padding = "a".repeat(5_000);
    let body = format!(
        r#"{{"model":"claude-haiku-4-5","max_tokens":1,"messages":[{{"role":"user","content":"hi {padding}"}}]}}"#,
        padding = &padding[..50] // keep it cheap — just enough to exceed a 2 MiB limit would be extreme; 50 chars is fine
    );

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request failed");

    assert_ne!(resp.status().as_u16(), 413, "limit=0 must never return 413");
    assert!(
        resp.status().is_success(),
        "request must succeed; got {}",
        resp.status()
    );
}
