# Step 11: Integration Tests — Upstream Timeout + Unreachable

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 2: integration tests. Both Wave 2 steps are independent and run in parallel. This step requires step-01 (MockResponse::Hang) for the timeout test.

### This Step
Creates `tests/integration/timeout.rs` with two tests:
1. **Timeout:** Proxy timeout set to 1 second; MockUpstream returns `Hang` (sleeps indefinitely). Test asserts response arrives within 5 seconds and status is 502 or 504.
2. **Unreachable:** MockUpstream is built, its URL captured, immediately shut down; proxy is pointed at that dead address via a new `upstream_url()` builder method. Test asserts proxy returns 502.

Both tests also require a small additive extension to `TestHarnessBuilder` (adding `upstream_url()`). Implement that first.

## Prerequisites
- step-01 complete — `MockResponse::Hang` and `MockUpstreamBuilder::hang()` must exist
- Wave 0 complete (all three of step-01, step-02, step-03)

## Files to Read Before Starting
- `tests/common/mock_upstream.rs` — confirm `MockResponse::Hang` and `.hang()` builder method exist
- `tests/common/harness.rs` — read the FULL file; you will extend `TestHarnessBuilder` here
- `tests/integration/mod.rs` — you will add `mod timeout;`

## Implementation

### Task 1: Add upstream_url() to TestHarnessBuilder in tests/common/harness.rs

The unreachable test needs to point the proxy at a dead address rather than the live mock. Add a field and builder method to `TestHarnessBuilder`.

**Add field to struct:**

```rust
pub struct TestHarnessBuilder {
    mock: Option<MockUpstream>,
    timeout_seconds: u64,
    ttl_seconds: u64,           // added by step-02
    upstream_url: Option<String>, // override proxy → upstream URL
}
```

**Initialize to `None` in `new()`:**

```rust
fn new() -> Self {
    Self {
        mock: None,
        timeout_seconds: 30,
        ttl_seconds: 0,
        upstream_url: None,
    }
}
```

**Add builder method** (after the `.ttl()` method):

```rust
/// Override the upstream URL the proxy forwards requests to (default: mock.url()).
/// Use to point the proxy at a dead address for unreachable-upstream tests.
pub fn upstream_url(mut self, url: String) -> Self {
    self.upstream_url = Some(url);
    self
}
```

**Update `build()` to use the override.** Replace the current line that reads `let mock_url = mock.url();` and its downstream use in `ServerConfig` with:

```rust
let mock_url = mock.url();
let upstream = self.upstream_url.unwrap_or_else(|| mock_url.clone());

let config = ServerConfig {
    addr: "127.0.0.1:0".parse().unwrap(),
    cache: cache.clone(),
    timeout_seconds: self.timeout_seconds,
    anthropic_upstream: Some(upstream.clone()),
    openai_upstream: Some(upstream.clone()),
    openrouter_upstream: Some(upstream.clone()),
    gemini_upstream: Some(upstream.clone()),
    stream_channel_capacity: 32,
};
```

**Verify the harness still compiles and existing tests pass before continuing:**

```
cargo nextest run --test spec
```

### Task 2: Create tests/integration/timeout.rs

```rust
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
```

### Task 3: Register in tests/integration/mod.rs

Add `mod timeout;` to `tests/integration/mod.rs`. The file currently has a placeholder comment. Replace the comment with the module declaration:

```rust
//! Integration tests — multi-request scenarios, SSE streaming, TTL.
//!
//! Run with: `cargo nextest run --test integration`

#[path = "../common/mod.rs"]
mod common;

mod timeout;
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run --test integration test_upstream_timeout_returns_gateway_error` exits 0
- [ ] `cargo nextest run --test integration test_upstream_unreachable_returns_502` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (harness change must not break existing spec tests)
- [ ] `grep 'upstream_url' tests/common/harness.rs` shows the field, init, method, and use in build()
- [ ] `grep 'mod timeout' tests/integration/mod.rs` produces output

## Reviewer Instructions

You are reviewing Step 11 implementation. Verify:

1. Run `cargo nextest run --test integration test_upstream_timeout_returns_gateway_error` — must exit 0, 1 test passed
2. Run `cargo nextest run --test integration test_upstream_unreachable_returns_502` — must exit 0, 1 test passed
3. Run `cargo nextest run --test spec` — must exit 0 (no regressions from harness.rs change)
4. Run `grep 'upstream_url' tests/common/harness.rs` — must show at least 4 occurrences (struct field, new(), method, build())

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: delete `tests/integration/timeout.rs`, remove `mod timeout;` from `tests/integration/mod.rs`, and revert the `upstream_url` additions in `tests/common/harness.rs`.
