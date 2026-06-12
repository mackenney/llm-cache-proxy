# Step 04: Coverage Gaps — Error Paths, Concurrency, Admin API, Known-Gap Probes

## Context

Steps 01–03 add cassette infrastructure and per-cassette tests. This step adds:
1. Error and edge-case paths not covered by the cassette scenarios
2. Concurrent request tests (the proxy must handle multiple simultaneous SSE streams)
3. Admin API coverage (`GET /stats`, `DELETE /stats`, `DELETE /cache`, `GET /`)
4. Bypass path tests
5. Explicit known-gap probe tests (surface failures against the vc_sse_19 #[ignore])
6. Cache correctness edge cases

Most of these are synthetic (no cassettes needed) — they extend `tests/integration/`
using `MockUpstream` directly.

## Prerequisites

- Steps 01–03 complete.

## Files to Read Before Starting

- `crates/lcp-server/SPEC.md` — all MUST clauses in §Proxy Behavior, §Admin Endpoints,
  §Extension Pipeline, §Bypass
- `tests/spec/` — existing spec tests (do NOT duplicate; fill gaps)
- `tests/integration/doppel.rs` — patterns to follow for integration tests
- `tests/common/harness.rs` — `TestHarness` API
- `AGENTS.md` — conventions

---

## 1. Error Path Tests (new file: `tests/integration/errors.rs`)

### 1a. Upstream 4xx/5xx not cached

```rust
/// 4xx and 5xx responses MUST NOT be written to cache.
/// Cache-miss counter must not increment on error responses.
#[tokio::test]
async fn upstream_4xx_not_cached() { ... }

#[tokio::test]
async fn upstream_5xx_not_cached() { ... }
```

- Queue a 429 in `MockUpstream`, send twice
- Assert: second request also hits the mock upstream (not cache)
- Assert: `x-lcp-cache: MISS` on both responses

### 1b. Upstream timeout

Already in `tests/integration/timeout.rs`. No new test needed.

### 1c. Partial SSE stream (stream dies mid-response)

```rust
/// If the upstream closes the connection mid-SSE-stream, the proxy must
/// forward what it received and not cache the partial response.
#[tokio::test]
async fn upstream_partial_sse_not_cached() {
    // MockUpstream: SSE with 3 chunks, then EOF with no [DONE]
    // Assert: client receives the 3 chunks
    // Assert: no cache write (partial streams are not valid cache entries)
    // Assert: a second identical request hits the upstream again (not a cached partial)
}
```

### 1d. Upstream returns non-SSE 200 (plain JSON) for a streaming request

```rust
/// Provider returns JSON when stream:true was requested (e.g., model overrides,
/// some Anthropic models in certain regions return non-stream).
/// Proxy must pass it through and cache correctly.
#[tokio::test]
async fn upstream_json_for_streaming_request_cached() { ... }
```

### 1e. Body too large

- SPEC: requests exceeding `--body-limit` must return 413 before proxy forwards
- Already covered by `tests/spec/body_limit.rs`; add one integration test that confirms
  mock upstream receives NO request when 413 fires

---

## 2. Concurrent Request Tests (new file: `tests/integration/concurrent.rs`)

These fill a real gap: the proxy handles many simultaneous SSE streams, each with its
own `SseRestoreStream` state machine. No concurrency bugs have been found yet, but they
need a coverage baseline.

### 2a. Concurrent identical requests (cache race)

```rust
/// Two identical requests in flight simultaneously. One will get MISS,
/// the other will also get MISS (not cached yet when both arrive).
/// Both must receive valid complete responses. After both complete,
/// a third identical request must get a HIT.
#[tokio::test]
async fn concurrent_identical_requests_both_complete() {
    // Queue 2 identical SSE responses in MockUpstream
    // Fire 2 concurrent requests via tokio::join!
    // Assert: both succeed (200, complete SSE stream)
    // Assert: mock received 2 requests (both were cache misses)
    // Assert: third request gets HIT (one of the two writes won the race)
}
```

### 2b. Concurrent different requests (independent state machines)

```rust
/// Two simultaneous requests to different models/prompts.
/// Each SseRestoreStream instance must maintain independent accumulator state.
#[tokio::test]
async fn concurrent_different_requests_independent_state() {
    // Queue 2 different SSE responses (different secrets in each)
    // Fire concurrently; assert each response contains its own secret
    // and NOT the other's secret
}
```

### 2c. Cache hit and miss in flight simultaneously

```rust
/// One request hits the cache (replay) while another hits the upstream.
/// Both must complete correctly.
#[tokio::test]
async fn concurrent_hit_and_miss() {
    // Pre-populate cache with one entry
    // Fire: one request that will HIT, one that will MISS
    // Concurrently
    // Assert: both complete with correct bytes
}
```

---

## 3. Admin API Coverage (new file: `tests/integration/admin.rs`)

### 3a. `GET /` — root endpoint

```rust
/// GET / returns 200 with server info.
#[tokio::test]
async fn admin_root_returns_200() { ... }
```

### 3b. `GET /stats`

```rust
/// Stats endpoint returns JSON with hit/miss counts.
#[tokio::test]
async fn stats_increments_on_requests() {
    // Make 1 miss + 1 hit
    // GET /stats → assert hits=1, misses=1 (or appropriate fields)
}
```

### 3c. `DELETE /stats`

```rust
/// DELETE /stats resets counters to zero.
#[tokio::test]
async fn stats_reset_clears_counters() {
    // Make 2 requests, DELETE /stats, GET /stats → assert 0/0
}
```

### 3d. `DELETE /cache`

```rust
/// DELETE /cache clears all stored entries.
/// A subsequent request that was previously cached must MISS again.
#[tokio::test]
async fn delete_cache_clears_entries() {
    // Fill cache, DELETE /cache, repeat original request → MISS
    // Assert: 2 upstream calls total (original + after clear)
}
```

### 3e. `GET /cache/{key}` — key inspection

```rust
/// GET /cache/<key> returns the stored entry.
/// Key is the x-lcp-key header from a prior MISS response.
#[tokio::test]
async fn cache_inspect_returns_stored_entry() {
    // Send request, capture x-lcp-key header from MISS response
    // GET /cache/{key} → 200 + JSON entry
    // Assert: entry contains non-empty body
}
```

---

## 4. Bypass Path Tests

Existing `tests/spec/bypass.rs` covers basic bypass. Add integration-level tests:

### 4a. Bypass header suppresses doppel

```rust
/// When x-lcp-bypass: 1 is set, DoppelExt must NOT run.
/// The secret in the request body must pass through unchanged.
#[tokio::test]
async fn bypass_suppresses_doppel_restore() {
    // Build harness with DoppelExt
    // Set x-lcp-bypass: 1 on request containing ANT
    // Mock upstream receives the ORIGINAL ANT (not swapped to fake)
    // Response body passes through without restoration attempt
}
```

### 4b. Bypass suppresses cache write

```rust
/// Bypassed requests are not cached.
#[tokio::test]
async fn bypass_suppresses_cache_write() {
    // Send bypassed request, then non-bypassed identical request
    // Assert: non-bypassed also gets MISS (bypass did not populate cache)
    // Assert: mock upstream received 2 requests
}
```

---

## 5. SSE Detection Edge Cases

### 5a. Spaceless `data:` in first chunk

```rust
/// First chunk starts with "data:{" (no space after colon).
/// is_sse_first_chunk must detect this as SSE.
/// Regression guard for the spaceless detection fix.
#[tokio::test]
async fn sse_detection_spaceless_data_prefix() {
    let chunks = vec![
        "data:{\"type\":\"message_start\",...}\n\n",
        "data:{\"type\":\"content_block_start\",...}\n\n",
        "data:{\"type\":\"message_stop\"}\n\n",
    ];
    // MockUpstream serves these as SSE
    // Assert: proxy routes to SseRestoreStream, not CollectingNonSse
    // Assert: response is forwarded correctly (all 3 frames present)
}
```

### 5b. Non-SSE 200 JSON (no SSE detection)

```rust
/// First chunk is a JSON object. Must NOT route to SseRestoreStream.
/// Verify proxy caches correctly as non-streaming response.
#[tokio::test]
async fn non_sse_json_response_cached_correctly() { ... }
```

### 5c. OpenRouter `: OPENROUTER PROCESSING` prefix

```rust
/// First chunk is ": OPENROUTER PROCESSING\n\n" (SSE comment).
/// is_sse_first_chunk must detect this correctly.
#[tokio::test]
async fn sse_detection_openrouter_comment_prefix() {
    let chunks = vec![
        ": OPENROUTER PROCESSING\n\n",
        ": OPENROUTER PROCESSING\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
        "data: [DONE]\n\n",
    ];
    // Assert: proxy detects SSE, forwards all chunks including comment lines
}
```

---

## 6. Known-Gap Surface Tests

These create FAILING external tests that surface known gaps per AGENTS.md policy:
"Bugs and gaps found during exploration MUST have a failing external test that
surfaces the problem before any fix is written."

Most of these tie into existing `#[ignore]` annotations.

### 6a. OpenAI Responses API `response.completed` body leaks fake

```rust
/// Known gap: response.completed event body contains the fake key
/// because extract_fields does not handle the response.completed payload.
///
/// SPEC.md §Known Gaps: "OpenAI Responses API response.completed event leaks fake"
///
/// This test will PASS once the gap is fixed. It documents current behavior.
#[tokio::test]
async fn known_gap_responses_completed_body_leaks_fake() {
    let c = Cassette::load("tests/fixtures/openai/responses_api_completed_body.toml");
    let (harness, _, fake_bytes) = cassette_harness(&c).await;
    // ... request ...
    let fake_in_completed = bytes.windows(fake_bytes.len()).any(|w| w == fake_bytes.as_slice());
    // Document — do not assert_absent, because the fake IS expected to appear
    eprintln!("response.completed body fake leak: {}", if fake_in_completed { "CONFIRMED" } else { "FIXED" });
    // The secret MUST still be present somewhere (delta events are restored)
    assert_present(&bytes, &[OPENAI_CLASSIC], "secret must appear via delta events");
}
```

### 6b. Gemini `finishReason` loss

Already documented in `cassette_gem_colocated_finish` (step-03). No new test needed.

### 6c. `response.output_text.done` body contains fake

```rust
/// Known gap probe: response.output_text.done body may contain the fake.
/// The done event carries the full assembled text; if the fake was in
/// the delta events, it appears here too.
///
/// This test is informational only (no assert_absent).
#[tokio::test]
async fn known_gap_probe_output_text_done_body() {
    let c = Cassette::load("tests/fixtures/openai/responses_api_output_text_done_body.toml");
    // ... same probe pattern ...
}
```

### 6d. DeepSeek R1 `delta.reasoning` field not extracted

```rust
/// DeepSeek R1 via OpenRouter uses delta.reasoning (not delta.reasoning_content).
/// The proxy's extract_fields only handles delta.reasoning_content.
/// If a secret appears in delta.reasoning, it is NOT restored.
///
/// This test uses the cassette to check whether the delta.reasoning field
/// passes through unchanged (no restoration, no corruption).
#[tokio::test]
async fn known_gap_probe_deepseek_reasoning_field() {
    let c = Cassette::load("tests/fixtures/openrouter/deepseek_r1_reasoning.toml");
    // ... request where secret appears in the reasoning prompt ...
    // Check: reasoning field content passes through (no crash, no corruption)
    // If secret was in reasoning: check whether fake leaks (document, do not fail)
    assert_present(&bytes, &[OPENAI_CLASSIC], "tool args secret must be restored");
    // Reasoning field analysis is informational
}
```

---

## 7. `tests/integration/mod.rs` Update

Register all new test modules:

```rust
mod admin;
mod cassettes;
mod concurrent;
mod errors;
```

---

## Acceptance Criteria

- [ ] All new test files compile: `cargo nextest run --test integration` exits 0
- [ ] `concurrent_identical_requests_both_complete` passes
- [ ] `bypass_suppresses_doppel_restore` passes (doppel NOT invoked on bypassed requests)
- [ ] `upstream_4xx_not_cached` and `upstream_5xx_not_cached` pass
- [ ] `stats_increments_on_requests` and `stats_reset_clears_counters` pass
- [ ] `delete_cache_clears_entries` passes
- [ ] `sse_detection_spaceless_data_prefix` passes (regression guard)
- [ ] `sse_detection_openrouter_comment_prefix` passes (regression guard)
- [ ] All known-gap probe tests compile and produce informational output (no hard failures)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] Total integration test count increases from current (58) to at least 90
  (37 cassette + ~35 new = 72 new tests; starting base ~58)

## Reviewer Instructions

- Confirm `concurrent_identical_requests_both_complete` does not flake (run it 5 times)
- Confirm `admin_root_returns_200` does not hardcode the proxy URL
- Confirm known-gap probe tests do not contain `assert_absent` for the fake
  (they probe and report, not fail)
- Confirm `bypass_suppresses_doppel_restore` checks the UPSTREAM received body
  (mock_requests()[0].body contains ANT, not the fake)
- Confirm `upstream_partial_sse_not_cached` queues a response with no `[DONE]` frame
  and a second identical request still goes to the upstream
