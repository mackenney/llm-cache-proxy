# Step 12: Integration Tests — TTL Expiry

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 2: integration tests. Both Wave 2 steps are independent and run in parallel. This step requires step-02 (TestHarnessBuilder::ttl()) to configure cache TTL.

### This Step
Creates `tests/integration/ttl.rs` with two tests that verify cache TTL semantics:
1. **TTL expiry:** A 1-second TTL is set; after 2 seconds the same request produces a MISS (entry expired). Covers SPEC.md §87–88.
2. **TTL=0 never expires:** TTL=0 means entries never expire; after 2 seconds the same request is still a HIT. Covers SPEC.md §86.

These tests use `tokio::time::sleep` and take ~2 seconds each, which is why they are in the integration tier.

## Prerequisites
- step-02 complete — `TestHarnessBuilder::ttl()` must be implemented in harness.rs
- Wave 0 complete (all three of step-01, step-02, step-03)

## Files to Read Before Starting
- `tests/common/harness.rs` — confirm `.ttl()` method exists and `Cache::open` uses `self.ttl_seconds` (added by step-02)
- `tests/integration/mod.rs` — you will add `mod ttl;`
- `SPEC.md` §86–88 — confirm TTL=0 means never-expire and TTL>0 means expire after N seconds

## Implementation

### Task 1: Create tests/integration/ttl.rs

```rust
//! TTL expiry integration tests.
//!
//! These tests sleep for 1-2 seconds to let entries expire, so they live in
//! the integration tier where timing-sensitive tests are acceptable.

use std::time::Duration;

use crate::common::{MockUpstream, TestHarness};

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"ttl-test"}]}"#
}

#[tokio::test]
async fn test_ttl_nonzero_entry_expires_after_ttl() {
    // TTL = 1 second.
    let mock = MockUpstream::builder()
        .sse(200, sse_response()) // first request: MISS
        .sse(200, sse_response()) // second request: MISS (expired)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).ttl(1).build().await;
    let client = reqwest::Client::new();

    // First request: MISS; entry stored with 1-second TTL.
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp1.headers().get("x-lcp-cache").expect("x-lcp-cache").as_bytes(),
        b"MISS",
        "first request must be a MISS"
    );
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Verify entry exists in cache.
    let entries_before = harness.cache().list_entries().unwrap();
    assert_eq!(entries_before.len(), 1, "one entry must exist after MISS");

    // Wait for TTL to expire (2 seconds > 1 second TTL).
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Second request: same body, but entry is expired — must be MISS.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let cache_header2 = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();
    let _ = resp2.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert_eq!(
        cache_header2, "MISS",
        "expired entry must produce a MISS; got: {cache_header2}"
    );

    // Stats: 1 miss total — the expired-row path returns Ok(None) without
    // incrementing the misses counter (only QueryReturnedNoRows does that).
    let stats = harness.cache().stats().unwrap();
    assert_eq!(
        stats.misses, 1,
        "only initial MISS increments counter; expired-row path does not; got: {}",
        stats.misses
    );
}

#[tokio::test]
async fn test_ttl_zero_entry_never_expires() {
    // TTL = 0 means never expire.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        // No second response queued — if the second request hits upstream, it gets a 500.
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).ttl(0).build().await;
    let client = reqwest::Client::new();

    // First request: MISS; entry stored with TTL=0 (never expires).
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Wait 2 seconds — entry must NOT expire with TTL=0.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Second request: same body — must still be a HIT.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(request_body())
        .send()
        .await
        .unwrap();
    let cache_header2 = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();

    assert_eq!(
        cache_header2, "HIT",
        "TTL=0 entry must not expire; expected HIT after 2s, got: {cache_header2}"
    );

    // Only 1 upstream call (the initial MISS).
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "only one upstream call expected — HIT must be served from cache"
    );
}
```

### Task 2: Register in tests/integration/mod.rs

Add `mod ttl;` to `tests/integration/mod.rs`. If step-11 has already added `mod timeout;`, append after it:

```rust
mod ttl;
```

If step-11 has not run yet (steps are parallel), add both `mod timeout;` and `mod ttl;` lines — but only add `mod ttl;` yourself; do not add `mod timeout;` (that belongs to step-11).

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run --test integration test_ttl_nonzero_entry_expires_after_ttl` exits 0
- [ ] `cargo nextest run --test integration test_ttl_zero_entry_never_expires` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions)
- [ ] `grep 'mod ttl' tests/integration/mod.rs` produces output

## Reviewer Instructions

You are reviewing Step 12 implementation. Verify:

1. Run `cargo nextest run --test integration test_ttl_nonzero_entry_expires_after_ttl` — must exit 0, test takes ~2s
2. Run `cargo nextest run --test integration test_ttl_zero_entry_never_expires` — must exit 0, test takes ~2s
3. Run `cargo nextest run --test spec` — must exit 0 (no regressions)
4. Run `grep 'mod ttl' tests/integration/mod.rs` — must produce output

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: delete `tests/integration/ttl.rs` and remove `mod ttl;` from `tests/integration/mod.rs`.
