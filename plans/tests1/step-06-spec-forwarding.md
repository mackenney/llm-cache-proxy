# Step 06: Spec Tests — Header Stripping

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 1: spec test files. All Wave 1 steps run in parallel after Wave 0 completes. This step verifies that hop-by-hop and client-specific headers are stripped before forwarding to upstream.

### This Step
Creates `tests/spec/forwarding.rs` with tests that send requests containing headers the proxy must strip (`Accept-Encoding`, `Host`, `Content-Length`, `Connection`) and assert that MockUpstream did not receive those headers. The proxy must strip these to avoid confusing upstream or revealing proxy internals. Covers SPEC.md §163–167 and lcp-server/SPEC.md §78–79.

`Accept-Encoding` stripping is verified here (not in `compression.rs`). The `compression.rs` file (step-10) tests request body decompression, a separate concern.

## Prerequisites
- Wave 0 complete (all three of step-01, step-02, step-03)

## Files to Read Before Starting
- `tests/common/mock_upstream.rs` — confirm `RecordedRequest.headers` field is a `HeaderMap`
- `tests/common/harness.rs` — confirm `harness.mock_requests()` returns `Vec<RecordedRequest>`
- `tests/spec/cache_miss.rs` — follow this pattern for harness setup
- `tests/spec/mod.rs` — you will add `mod forwarding;`

## Implementation

### Task 1: Create tests/spec/forwarding.rs

```rust
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
```

### Task 2: Register in tests/spec/mod.rs

Add `mod forwarding;` to `tests/spec/mod.rs`:

```rust
mod forwarding;
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run --test spec test_accept_encoding_stripped_before_forwarding` exits 0
- [ ] `cargo nextest run --test spec test_host_header_stripped_before_forwarding` exits 0
- [ ] `cargo nextest run --test spec test_connection_header_stripped_before_forwarding` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions)
- [ ] `grep 'mod forwarding' tests/spec/mod.rs` produces output

## Reviewer Instructions

You are reviewing Step 06 implementation. Verify:

1. Run `cargo nextest run --test spec test_accept_encoding_stripped_before_forwarding` — must exit 0
2. Run `cargo nextest run --test spec test_host_header_stripped_before_forwarding` — must exit 0
3. Run `cargo nextest run --test spec test_connection_header_stripped_before_forwarding` — must exit 0
4. Run `cargo nextest run --test spec` — must exit 0 (all spec tests pass)

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: delete `tests/spec/forwarding.rs` and remove `mod forwarding;` from `tests/spec/mod.rs`.
