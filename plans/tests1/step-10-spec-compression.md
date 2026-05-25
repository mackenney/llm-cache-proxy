# Step 10: Spec Tests — Compressed Request Body Decompression

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 1: spec test files. All Wave 1 steps run in parallel after Wave 0 completes. This step depends on step-03 (flate2 dev-dep) being complete before it can compile.

### This Step
Creates `tests/spec/compression.rs` with a single test verifying that gzip-compressed request bodies are decompressed before cache key hashing. A gzip-compressed request and an identical plain-text request must produce the same cache key (the second is a HIT). Covers SPEC.md §168–169.

**Scope note:** Header stripping (`Accept-Encoding`) is tested in `forwarding.rs` (step-06). This file tests only request body decompression.

## Prerequisites
- Wave 0 complete — specifically step-03 (flate2 added to tests/Cargo.toml) must be done before this step compiles

## Files to Read Before Starting
- `tests/Cargo.toml` — confirm `flate2 = "1"` is present (added by step-03)
- `tests/spec/cache_miss.rs` — follow this pattern for harness setup
- `tests/spec/mod.rs` — you will add `mod compression;`
- `SPEC.md` §168–169 — confirm decompression-before-hashing requirement

## Implementation

### Task 1: Create tests/spec/compression.rs

```rust
//! Compression tests — SPEC contract: compressed request body is decompressed before cache key hashing.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::common::{MockUpstream, TestHarness};

fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn sse_response() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

#[tokio::test]
async fn test_compressed_request_body_decompressed_before_hashing() {
    // Two SSE responses queued — if the second request is a cache HIT, the second
    // queued response is never consumed. If it is a MISS, both are consumed.
    let mock = MockUpstream::builder()
        .sse(200, sse_response())
        .sse(200, sse_response())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let plain_body =
        r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"compress-test"}]}"#;
    let compressed_body = gzip_compress(plain_body.as_bytes());

    // First request: gzip-compressed body.
    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .header("content-encoding", "gzip")
        .body(compressed_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);
    let cache_header1 = resp1
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_header1, "MISS", "first request must be MISS");
    let _ = resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    // Second request: identical plain-text body — must be a HIT.
    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(plain_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);
    let cache_header2 = resp2
        .headers()
        .get("x-lcp-cache")
        .expect("x-lcp-cache must be present")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        cache_header2, "HIT",
        "plain-body request identical to earlier compressed request must be a cache HIT — \
         compressed and plain bodies must hash to the same key"
    );

    // Upstream must have been called exactly once (the compressed MISS);
    // the HIT was served from cache.
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "upstream must be called exactly once — HIT must be served from cache"
    );
}
```

### Task 2: Register in tests/spec/mod.rs

Add `mod compression;` to `tests/spec/mod.rs`:

```rust
mod compression;
```

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `grep 'flate2' tests/Cargo.toml` outputs `flate2 = "1"` (confirming step-03 prerequisite)
- [ ] `cargo nextest run --test spec test_compressed_request_body_decompressed_before_hashing` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions)
- [ ] `grep 'mod compression' tests/spec/mod.rs` produces output

## Reviewer Instructions

You are reviewing Step 10 implementation. Verify:

1. Run `grep 'flate2' tests/Cargo.toml` — must output `flate2 = "1"`
2. Run `cargo nextest run --test spec test_compressed_request_body_decompressed_before_hashing` — must exit 0, 1 test passed
3. Run `cargo nextest run --test spec` — must exit 0 (all spec tests pass)
4. Confirm `tests/spec/compression.rs` does NOT contain `test_accept_encoding_stripped_before_forwarding` (that test belongs to `forwarding.rs`)

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: delete `tests/spec/compression.rs` and remove `mod compression;` from `tests/spec/mod.rs`.
