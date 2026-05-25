# Step 05: Spec Tests — Bypass Behavior

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 1: spec test files. All Wave 1 steps run in parallel after Wave 0 completes. This step creates the bypass spec tests.

### This Step
Creates `tests/spec/bypass.rs` with tests verifying the `x-lcp-bypass: 1` header behavior: the response carries `x-lcp-cache: BYPASS`, no `x-lcp-key` header is set, the upstream is called on every bypass request (no caching), and bypass requests are not recorded in trace entries. Covers SPEC.md §154–159 and §204.

## Prerequisites
- Wave 0 complete (all three of step-01, step-02, step-03)

## Files to Read Before Starting
- `tests/spec/cache_miss.rs` — follow this exact pattern for harness setup and assertion style
- `tests/spec/mod.rs` — you will add `mod bypass;`
- `SPEC.md` §154–159, §204 — confirm bypass header names and trace exclusion requirement

## Implementation

### Task 1: Create tests/spec/bypass.rs

```rust
//! Bypass tests — SPEC contract: x-lcp-bypass:1 skips cache read/write.

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hello"}]}"#
}

#[tokio::test]
async fn test_bypass_returns_x_lcp_cache_bypass() {
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
        .header("x-lcp-bypass", "1")
        .body(request_body())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let cache_header = resp
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache header must be present on bypass");
    assert_eq!(
        cache_header, "BYPASS",
        "bypass response must set x-lcp-cache: BYPASS"
    );
}

#[tokio::test]
async fn test_bypass_omits_x_lcp_key() {
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
        .header("x-lcp-bypass", "1")
        .body(request_body())
        .send()
        .await
        .unwrap();

    assert!(
        resp.headers().get("x-lcp-key").is_none(),
        "bypass response must not set x-lcp-key"
    );
}

#[tokio::test]
async fn test_bypass_upstream_called_every_time() {
    // Queue two identical SSE responses — both requests hit the upstream.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    for _ in 0..2 {
        let resp = client
            .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
            .header("x-api-key", "test-key")
            .header("content-type", "application/json")
            .header("x-lcp-bypass", "1")
            .body(request_body())
            .send()
            .await
            .unwrap();
        let _ = resp.bytes().await.unwrap();
    }

    assert_eq!(
        harness.mock_requests().len(),
        2,
        "bypass must call upstream on every request, not serve from cache"
    );
}

#[tokio::test]
async fn test_bypass_does_not_cache() {
    // First request: bypass. Second request: no bypass, same body.
    // If bypass had cached, the second request would be a HIT. It must be MISS.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // First: bypass request
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("x-lcp-bypass", "1")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Second: identical request without bypass — must be MISS, not HIT.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();

    let cache_header = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache header");
    assert_eq!(
        cache_header, "MISS",
        "request after bypass must be MISS (bypass must not populate cache)"
    );
}

#[tokio::test]
async fn test_bypass_not_recorded_in_trace() {
    // A bypass request with a trace ID must not appear in GET /trace/<id>.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // Bypass request with trace ID.
    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("x-lcp-bypass", "1")
        .header("x-lcp-trace", "bypass-trace-123")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Query trace — entries must be empty because bypass is not recorded.
    let trace_resp = client
        .get(format!("{}/trace/bypass-trace-123", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(trace_resp.status(), 200);
    let body: serde_json::Value = trace_resp.json().await.unwrap();
    let entries = body["entries"].as_array().expect("entries must be an array");
    assert_eq!(
        entries.len(),
        0,
        "bypass request must not appear in trace entries; got: {entries:?}"
    );

    // Contrast: non-bypass request with a different trace ID records 1 entry.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("x-lcp-trace", "normal-trace-456")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let trace_resp2 = client
        .get(format!("{}/trace/normal-trace-456", harness.proxy_url()))
        .send()
        .await
        .unwrap();
    let body2: serde_json::Value = trace_resp2.json().await.unwrap();
    let entries2 = body2["entries"].as_array().expect("entries must be an array");
    assert_eq!(
        entries2.len(),
        1,
        "non-bypass request must appear in trace entries"
    );
}
```

### Task 2: Register in tests/spec/mod.rs

Add `mod bypass;` to `tests/spec/mod.rs`:

```rust
mod bypass;
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run --test spec test_bypass_returns_x_lcp_cache_bypass` exits 0
- [ ] `cargo nextest run --test spec test_bypass_omits_x_lcp_key` exits 0
- [ ] `cargo nextest run --test spec test_bypass_upstream_called_every_time` exits 0
- [ ] `cargo nextest run --test spec test_bypass_does_not_cache` exits 0
- [ ] `cargo nextest run --test spec test_bypass_not_recorded_in_trace` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions)

## Reviewer Instructions

You are reviewing Step 05 implementation. Verify:

1. Run `cargo nextest run --test spec test_bypass` — must exit 0, 5 tests passed
2. Run `cargo nextest run --test spec` — must exit 0 (all spec tests pass)
3. Run `grep 'mod bypass' tests/spec/mod.rs` — must produce output

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: delete `tests/spec/bypass.rs` and remove `mod bypass;` from `tests/spec/mod.rs`.
