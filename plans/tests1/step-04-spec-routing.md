# Step 04: Spec Tests — Routing (Unknown Provider → 404)

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 1: spec test files. All Wave 1 steps depend on Wave 0 completing first (ensures the codebase compiles with Hang + TTL + flate2 changes). This step and all other Wave 1 steps run in parallel after Wave 0.

### This Step
Creates `tests/spec/routing.rs` with tests that verify the proxy returns 404 with a meaningful body for unknown provider prefixes (e.g., `/badprovider/v1/messages`). Covers SPEC.md routing requirements and lcp-server/SPEC.md §53.

## Prerequisites
- Wave 0 complete (all three of step-01, step-02, step-03)

## Files to Read Before Starting
- `tests/spec/cache_miss.rs` — follow this exact pattern for test structure and harness setup
- `tests/spec/mod.rs` — to see existing module registrations; you will add `mod routing;`
- `SPEC.md` or `crates/lcp-server/SPEC.md` — confirm 404 behavior for unknown providers

## Implementation

### Task 1: Create tests/spec/routing.rs

```rust
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
        body.to_lowercase().contains("unknown") || body.to_lowercase().contains("provider") || body.to_lowercase().contains("not found"),
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

    assert_ne!(resp.status(), 404, "valid anthropic prefix must not return 404");
}
```

### Task 2: Register in tests/spec/mod.rs

Add `mod routing;` to `tests/spec/mod.rs`. The file currently ends with `mod model_extraction;`. Append after it:

```rust
mod routing;
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run --test spec test_unknown_provider_returns_404` exits 0
- [ ] `cargo nextest run --test spec test_valid_provider_does_not_return_404` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions in existing tests)
- [ ] `grep 'mod routing' tests/spec/mod.rs` shows the module registration

## Reviewer Instructions

You are reviewing Step 04 implementation. Verify:

1. Run `cargo nextest run --test spec test_unknown_provider_returns_404` — must exit 0, 1 test passed
2. Run `cargo nextest run --test spec test_valid_provider_does_not_return_404` — must exit 0, 1 test passed
3. Run `cargo nextest run --test spec` — must exit 0 (all spec tests pass)
4. Run `grep 'mod routing' tests/spec/mod.rs` — must produce output

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: delete `tests/spec/routing.rs` and remove `mod routing;` from `tests/spec/mod.rs`.
