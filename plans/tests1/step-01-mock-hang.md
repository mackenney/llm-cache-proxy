# Step 01: Add MockResponse::Hang Variant

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 0: infrastructure extension. This step provides the `MockResponse::Hang` variant needed by the timeout integration test in Wave 2. All Wave 0 steps are independent and run in parallel.

### This Step
Adds a `MockResponse::Hang` variant to `tests/common/mock_upstream.rs`. When the mock pops a `Hang` response, it sleeps for 1 hour before returning, simulating an upstream that never responds. This enables the timeout test to assert that the proxy returns 5xx within its configured timeout window.

## Prerequisites
- None (Wave 0 step)

## Files to Read Before Starting
- `tests/common/mock_upstream.rs` — full file; understand the `MockResponse` enum, `handle_request`, `MockUpstreamBuilder`

## Implementation

### Task 1: Add Hang variant to MockResponse enum

In `tests/common/mock_upstream.rs`, extend the `MockResponse` enum (currently at line ~36) with a new variant:

```rust
pub enum MockResponse {
    /// JSON response with status and body.
    Json { status: u16, body: String },
    /// SSE response with status and chunks (each chunk is a complete SSE frame).
    Sse { status: u16, chunks: Vec<String> },
    /// Error response with status and body.
    Error { status: u16, body: String },
    /// Never responds — sleeps indefinitely to simulate a hung upstream.
    /// Use with a short proxy timeout to test gateway timeout behavior.
    Hang,
}
```

### Task 2: Handle Hang in handle_request

In the `match resp` block inside `handle_request` (after the `MockResponse::Error` arm), add:

```rust
MockResponse::Hang => {
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    StatusCode::OK.into_response()
}
```

### Task 3: Add hang() builder method

In `MockUpstreamBuilder` impl (after the `error()` method), add:

```rust
/// Queue a response that never arrives — sleeps 1 hour before replying.
/// Pair with a short proxy timeout to test gateway error behavior.
pub fn hang(self) -> Self {
    self.response(MockResponse::Hang)
}
```

### Task 4: Add unit test

In the `#[cfg(test)] mod tests` block at the bottom of `mock_upstream.rs`, add a test that verifies a `Hang` response actually hangs and a client with a short timeout sees the timeout:

```rust
#[tokio::test]
async fn mock_hang_causes_client_timeout() {
    let mock = MockUpstream::builder().hang().build().await;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(200))
        .build()
        .unwrap();

    let result = client.get(format!("{}/hang", mock.url())).send().await;
    assert!(result.is_err(), "expected timeout error, got: {:?}", result);
}
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo build --test spec` exits 0 (codebase compiles with changes)
- [ ] `cargo build --test integration` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions in existing spec tests)
- [ ] `cargo nextest run -p lcp-tests mock_hang_causes_client_timeout` exits 0
- [ ] `grep -n 'Hang' tests/common/mock_upstream.rs` shows the variant in enum, the match arm, and the builder method

## Reviewer Instructions

You are reviewing Step 01 implementation. Verify:

1. Run `cargo build --test spec` — must exit 0
2. Run `cargo nextest run -p lcp-tests mock_hang_causes_client_timeout` — must exit 0 and show 1 test passed
3. Run `grep -n 'Hang' tests/common/mock_upstream.rs` — must show at least 3 occurrences (enum, match arm, builder method)
4. Run `cargo nextest run --test spec` — must exit 0 (no regressions)

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: `git revert` the commit for step-01. The `Hang` variant, its match arm, and the builder method are all additions — removal restores the original state.
