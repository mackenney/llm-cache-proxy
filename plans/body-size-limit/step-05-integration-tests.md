# Step 05: Add Integration Tests for Body Limit

## Context

### Overall Objective
Add a configurable body size limit to lcp to prevent Axum's default 2 MiB limit from rejecting large LLM requests with images or long context.

### Phase Context
Wave 2 adds tests. This step adds integration tests that verify:
- Requests exceeding the limit get HTTP 413
- Requests at or below the limit pass through normally

### This Step
Create `tests/integration/body_limit.rs` with two tests:
1. `test_body_exceeding_limit_returns_413` — send a body larger than the configured limit, expect 413
2. `test_body_within_limit_passes_through` — send a body within the limit, expect normal proxy behavior

## Prerequisites
- Step 02 complete (`DefaultBodyLimit` layer applied in router)
- Step 03 complete (CLI wiring done)
- Step 04 complete (harness has `.body_limit()` builder method)

## Files to Read Before Starting
- `tests/integration/timeout.rs` — pattern for integration tests
- `tests/common/harness.rs:139-165` — `build()` method showing `ServerConfig` construction
- `tests/integration/mod.rs` — module declarations

## Implementation

### Task 1: Add `mod body_limit;` to `tests/integration/mod.rs`

Add after the existing module declarations:

```rust
mod body_limit;
```

### Task 2: Create `tests/integration/body_limit.rs`

Create the file with the following content:

```rust
//! Body limit integration tests.
//!
//! Verify that requests exceeding the configured body limit are rejected
//! with HTTP 413 before reaching the proxy handler.

use crate::common::{MockUpstream, TestHarness};

/// Request body larger than the limit MUST return 413.
#[tokio::test]
async fn test_body_exceeding_limit_returns_413() {
    // Set a small limit: 1024 bytes
    let mock = MockUpstream::builder()
        .json(200, r#"{"content":"should not reach"}"#)
        .build()
        .await;

    let harness = TestHarness::builder()
        .mock(mock)
        .body_limit(1024)
        .build()
        .await;

    let client = reqwest::Client::new();

    // Send a body of 2048 bytes — exceeds the 1024-byte limit
    let large_body = "x".repeat(2048);

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(large_body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        413,
        "body exceeding limit must return 413 Payload Too Large"
    );
}

/// Request body within the limit MUST pass through to upstream.
#[tokio::test]
async fn test_body_within_limit_passes_through() {
    let mock = MockUpstream::builder()
        .json(200, r#"{"id":"msg_123","content":[{"text":"hello"}],"model":"claude-sonnet-4-20250514","type":"message"}"#)
        .build()
        .await;

    // Set limit: 4096 bytes
    let harness = TestHarness::builder()
        .mock(mock)
        .body_limit(4096)
        .build()
        .await;

    let client = reqwest::Client::new();

    // Send a body of 1024 bytes — well within the 4096-byte limit
    let body = format!(
        r#"{{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{{"role":"user","content":"{}"}}]}}"#,
        "x".repeat(900) // pad to make body ~1KB
    );

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "body within limit must pass through; got {}",
        resp.status()
    );
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:
- [ ] `cargo nextest run --test integration body_limit` exits with code 0
- [ ] `cargo clippy --tests -- -D warnings` exits with code 0
- [ ] `grep -c 'async fn test_' tests/integration/body_limit.rs` outputs `2` (two tests exist)

## Reviewer Instructions

You are reviewing Step 05. Verify:
1. Run `cargo nextest run --test integration body_limit` — must exit 0 (both tests pass)
2. Run `cargo clippy --tests -- -D warnings` — must exit 0
3. Check `tests/integration/mod.rs` includes `mod body_limit;`
4. Check `tests/integration/body_limit.rs`:
   - `test_body_exceeding_limit_returns_413` configures `.body_limit(1024)` and sends 2048-byte body, asserts 413
   - `test_body_within_limit_passes_through` configures `.body_limit(4096)` and sends ~1KB body, asserts 200

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong>"

## Rollback
`git revert HEAD` if this is the most recent commit, otherwise identify the commit with message "step-05: integration-tests" and revert it.
