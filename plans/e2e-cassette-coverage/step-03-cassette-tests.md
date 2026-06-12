# Step 03: Cassette-Based Integration Tests

## Context

### Where in the Tier

These are **integration tests** (`tests/integration/`), not spec or E2E tests. They sit
between the unit-level spec tests (synthetic fixtures, no network) and E2E tests (live API,
no CI gate):

| Tier | Source | Network | CI gate | Deterministic |
|---|---|---|---|---|
| `tests/spec/` | Synthetic | No | Always | Yes |
| `tests/integration/` | Cassette (real-captured) | No | Always | Yes |
| `tests/e2e/` | Live API | Yes | Manual / env-gated | No |

Cassette tests become part of the always-on CI suite. They cover real wire formats
without requiring API credentials.

### File Layout

```
tests/integration/
  cassettes.rs      (NEW — cassette-based tests)
  doppel.rs         (existing)
  ...
```

## Test Structure

Each test loads a cassette, builds a `TestHarness` with `MockUpstream::Recorded`, sends
the request through the proxy (with the synthetic secret embedded), and asserts the
response bytes.

### Shared helper (top of `cassettes.rs`)

```rust
//! Cassette-based integration tests: real provider wire formats, no live API.
//!
//! Each test loads a captured cassette from tests/fixtures/, feeds it through
//! the proxy via MockUpstream::Recorded, and asserts restoration correctness.
//! These tests run in CI without API keys.

use doppel::{patterns, swap as doppel_swap};
use bytes::Bytes;
use lcp_server::{DoppelExt, ExtensionPipeline};
use crate::common::{Cassette, MockUpstream, TestHarness};

/// Synthetic secrets (same as tests/e2e/sse_fields.rs — NOT real credentials).
const ANT:           &[u8] = b"sk-ant-api03-YLY9P1-i5dC1zbDHjPYKuQHRM0TsEXQj6wiLZGOvUCYMDV25RlbUUTO1bZ_tbvx0OMdtvzVCSDh6vkpciXKbN_5lGMcOQAA";
const OPENAI_CLASSIC: &[u8] = b"sk-v0zsmdzWwRZktfsJIdQWQvKdIYk1LYrtuF3hWeJep2YvHzQ3";
const GCP:           &[u8] = b"AIzavURt9l4GMP5k339tqrQWeHPJqdXRArxL-xi";

fn secret_for_kind(kind: &str) -> &'static [u8] {
    match kind {
        "anthropic"     => ANT,
        "openai_classic" => OPENAI_CLASSIC,
        "gcp"           => GCP,
        other => panic!("unknown secret_kind: {other}"),
    }
}

fn pattern_for_kind(kind: &str) -> doppel::Pattern {
    match kind {
        "anthropic"     => patterns::anthropic(),
        "openai_classic" => patterns::openai_classic(),
        "gcp"           => patterns::gcp(),
        other => panic!("unknown pattern kind: {other}"),
    }
}

/// Build a TestHarness that replays a cassette, with DoppelExt wired.
async fn cassette_harness(c: &Cassette) -> (TestHarness, Vec<u8>, Vec<u8>) {
    let pat = pattern_for_kind(&c.secret_kind);
    let secret = secret_for_kind(&c.secret_kind);
    let sr = doppel_swap(secret, &[pat.clone()]).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let mock = MockUpstream::builder().cassette(c).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;

    (harness, secret.to_vec(), fake_bytes)
}
```

---

## Test Definitions (one per cassette)

### Anthropic Tests

#### `cassette_ant_tool_use_input_json`

```rust
/// Anthropic tool call: fake key embedded in input_json_delta chunks.
/// Verifies the flush_safe_prefix boundary fix: JSON object wrapping
/// ("{"key": "sk") must not split the fake across flush boundaries.
#[tokio::test]
async fn cassette_ant_tool_use_input_json() {
    let c = Cassette::load("tests/fixtures/anthropic/tool_use_input_json.toml");
    let (harness, secret, fake_bytes) = cassette_harness(&c).await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "claude-haiku-4-5", "max_tokens": 300, "stream": true,
        "tools": [{"name":"store_key","description":"Store key",
                   "input_schema":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}],
        "messages": [{"role":"user","content":
            format!("Call store_key with this key: {}", String::from_utf8_lossy(ANT))}]
    }).to_string();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send().await.unwrap();

    assert!(resp.status().is_success());
    let bytes = resp.bytes().await.unwrap();

    // Secret present (restore succeeded), fake absent (no leak)
    assert_present(&bytes, &[ANT], "tool_use: secret must be present");
    assert_absent(&bytes, &[&fake_bytes], "tool_use: fake must be absent");

    // Verify the cache was written (cache miss, not bypassed)
    harness.wait_for_writes().await;
    assert_eq!(harness.cache().miss_count().await, 1);
}
```

#### `cassette_ant_thinking_and_text`
- Asserts: `ANT` present, fake absent
- Also asserts: `"thought":true` or `"thought": true` in response (thinking block actually arrived)
- Also asserts: `"content_block_stop"` appears in response AFTER restored thinking text
  (verifies block-scope flush ordering for thinking block)

#### `cassette_ant_multi_block_stop`
- Two simultaneous tool calls (index 0 and index 1)
- Asserts: `ANT` present in outputs for BOTH tool calls
- Asserts: `"content_block_stop"` for index 0 appears after index-0 restored content
- Asserts: `"content_block_stop"` for index 1 appears after index-1 restored content
  (VC-SSE-16 block isolation)

#### `cassette_ant_text_only`
- Plain text reply, no tool call
- Asserts: response contains `ANT`, fake absent
- Asserts: `"message_stop"` present
- Verifies: cache write happened

#### `cassette_ant_message_delta_stop`
- The model emits `message_delta` (with stop_reason) BEFORE `message_stop`
- Asserts: `ANT` appears BEFORE `"message_delta"` appears in concatenated bytes
  (VC-SSE-15 message_delta ordering)

#### `cassette_ant_empty_thinking`
- Model enabled thinking but produced no thought
- Asserts: `ANT` present (fake absent) — text content still restored correctly
- Asserts: No "thinking" key in output (graceful empty-block handling)

#### `cassette_ant_error_rate_limit`
- Upstream returns 429
- Asserts: proxy returns 429 to client
- Asserts: cache miss count = 0 (error responses not cached)
- Asserts: `harness.cache().miss_count() == 0` (cache write suppressed)

#### `cassette_ant_error_overloaded`
- Upstream returns 529
- Same assertions as 429 test

---

### OpenAI Chat Completions Tests

#### `cassette_oai_tool_calls`
- Asserts: `OPENAI_CLASSIC` present, fake absent
- Asserts: `"finish_reason"` appears AFTER restored content in bytes
  (VC-SSE-17 finish_reason ordering)
- Asserts: `"[DONE]"` appears AFTER `"finish_reason"` appears

#### `cassette_oai_colocated_finish`
```rust
/// The "content: empty string co-located with finish_reason" bug scenario.
/// Real OpenAI/OpenRouter wire format. Regression guard for:
///   fix: skip empty-string content to unblock classify_terminal (commit 0bc6827)
/// If extract_fields returns content:"" as extractable, classify_terminal never
/// fires and the terminal flush doesn't happen before finish_reason.
#[tokio::test]
async fn cassette_oai_colocated_finish() {
    let c = Cassette::load("tests/fixtures/openai/chat_tool_calls_colocated_finish.toml");
    let (harness, _secret, fake_bytes) = cassette_harness(&c).await;
    // ... (same request pattern)
    assert_present(&bytes, &[OPENAI_CLASSIC], "colocated: secret must be present");
    assert_absent(&bytes, &[&fake_bytes], "colocated: fake must NOT appear");
    // If the fix regresses, the fake leaks to the client and fake_bytes is found
}
```

#### `cassette_oai_content_only`
- Plain text, no tool
- Asserts: `OPENAI_CLASSIC` present in text output, fake absent
- Asserts: `"finish_reason"` in stream

#### `cassette_oai_o4mini_reasoning`
- `reasoning_content` field in delta
- Asserts: `OPENAI_CLASSIC` present, fake absent
- Asserts: `"reasoning_content"` appears in output (reasoning preserved)

#### `cassette_oai_multi_tool`
- Two tool calls in parallel (tool_calls index 0 and index 1)
- Asserts: `OPENAI_CLASSIC` present TWICE (once per tool call argument)
- Asserts: fake absent

#### `cassette_oai_stream_error`
- 400 response from upstream
- Asserts: proxy returns 400
- Asserts: no cache write

#### `cassette_oai_finish_stop`
- Content delta + `finish_reason: stop` + `[DONE]`
- Asserts: `"finish_reason":"stop"` appears after `OPENAI_CLASSIC` in bytes
- Asserts: `"[DONE]"` appears after `"finish_reason"`

---

### OpenAI Responses API Tests

#### `cassette_oai_resp_text_delta`
```rust
/// Responses API: fake key in output_text.delta chunks.
/// Also verifies synthetic frames use correct event type name "response.output_text.delta"
/// (regression guard for commit f6aeeca: synthetic frames used wrong "output_text").
#[tokio::test]
async fn cassette_oai_resp_text_delta() {
    // ...
    assert_present(&bytes, &[OPENAI_CLASSIC], "resp text: secret present");
    assert_absent(&bytes, &[&fake_bytes], "resp text: fake absent");
    // Verify synthetic frames use correct event type (the f6aeeca fix)
    assert!(bytes.windows(b"response.output_text.delta".len())
        .any(|w| w == b"response.output_text.delta"),
        "synthetic frames must use event: response.output_text.delta, not event: output_text");
}
```

#### `cassette_oai_resp_done_sequence`
- Full real event sequence: delta→done→content_part.done→output_item.done→completed
- Asserts for each terminal: `OPENAI_CLASSIC` appears before the terminal in bytes
  (VC-SSE-18 sub-cases A, B, C — with real captured data now backing them)

#### `cassette_oai_resp_error_incomplete`
- `response.incomplete` event (max_output_tokens hit)
- Asserts: `OPENAI_CLASSIC` present before `"response.incomplete"` in bytes
  (VC-SSE-18b regression guard with real wire data)

#### `cassette_oai_resp_output_text_done_body`
```rust
/// Known-gap probe: response.output_text.done carries the full assembled text.
/// The fake MAY appear in the done body — this test DOCUMENTS current behavior.
/// If the fake is absent, restoration is already working (great).
/// If the fake is present, this confirms the known gap (response.output_text.done
/// body not restored) and the test should be #[ignore] with a clear gap description.
#[tokio::test]
async fn cassette_oai_resp_output_text_done_body() {
    // ... setup ...
    // DO NOT assert_absent here — this is a known-gap probe
    let done_body_contains_fake = bytes.windows(fake_bytes.len())
        .any(|w| w == fake_bytes.as_slice());
    if done_body_contains_fake {
        // Document that the body of output_text.done leaks the fake.
        // This is expected under the current known gap. The test passes either way.
        eprintln!("KNOWN GAP CONFIRMED: response.output_text.done body contains fake key");
    } else {
        // If the gap was fixed, the fake is absent — assert it stays absent.
        assert_absent(&bytes, &[&fake_bytes], "resp done body: fake must be absent (gap fixed)");
    }
    // Either way: secret IS present somewhere in the response
    assert_present(&bytes, &[OPENAI_CLASSIC], "resp done body: secret must be present");
}
```

#### `cassette_oai_resp_completed_body`
- Same pattern as above — known-gap probe for `response.completed` body
- Document whether `response.completed` body contains the fake

---

### OpenRouter Tests

#### `cassette_or_claude_tool_use`
- Same assertions as `cassette_ant_tool_use_input_json` but via OpenRouter
- Also asserts: `: OPENROUTER PROCESSING` comment lines are present in output
  (proxy must pass through these SSE comment frames)

#### `cassette_or_claude_finish_colocated`
```rust
/// THE Jun 2026 bug scenario: OpenRouter Claude sends content:"" + finish_reason.
/// This test is the regression guard that would have caught the bug before E2E.
/// The cassette is captured from real OpenRouter+Claude traffic.
///
/// Regression guard for: fix: skip empty-string content (commit 0bc6827)
#[tokio::test]
async fn cassette_or_claude_finish_colocated() {
    let c = Cassette::load("tests/fixtures/openrouter/claude_haiku_finish_colocated.toml");
    let (harness, _secret, fake_bytes) = cassette_harness(&c).await;
    // ... (tool call request with OPENAI_CLASSIC embedded)
    assert_present(&bytes, &[OPENAI_CLASSIC], "or_colocated: secret must be present");
    assert_absent(&bytes, &[&fake_bytes], "or_colocated: fake MUST be absent — if this fails, the empty-content fix regressed");
}
```

#### `cassette_or_claude_text`
- Plain text response via OpenRouter (OpenRouter normalizes Anthropic → Chat Completions)
- Asserts: correct event format (OpenAI-shaped, not Anthropic-shaped)
- Asserts: OPENAI_CLASSIC present, fake absent

#### `cassette_or_deepseek_chat`
- DeepSeek Chat via OpenRouter (SiliconFlow provider)
- Asserts: OPENAI_CLASSIC present, fake absent
- Asserts: `"finish_reason"` appears in stream
- Asserts: usage frame present before `[DONE]` (DeepSeek-specific)

#### `cassette_or_deepseek_r1_reasoning`
```rust
/// DeepSeek R1 via OpenRouter sends delta.reasoning (NOT delta.reasoning_content).
/// This test verifies the proxy handles both field names.
/// IMPORTANT: if delta.reasoning is not extracted, the restore still works
/// (reasoning doesn't contain the secret in the tool-call scenario), but
/// the proxy should at least not break the stream.
#[tokio::test]
async fn cassette_or_deepseek_r1_reasoning() {
    // ...
    assert_present(&bytes, &[OPENAI_CLASSIC], "deepseek_r1: secret present in tool args");
    assert_absent(&bytes, &[&fake_bytes], "deepseek_r1: fake absent");
    // Verify reasoning field is present (not stripped by restore)
    assert!(bytes.windows(b"\"reasoning\"".len()).any(|w| w == b"\"reasoning\""),
        "deepseek_r1: reasoning field must pass through");
}
```

#### `cassette_or_processing_prefix`
- First response chunk is `: OPENROUTER PROCESSING` (SSE comment line)
- Asserts: proxy routes this correctly as SSE (not as non-SSE JSON)
- Asserts: comment lines appear in proxy output unchanged
- Regression guard for: `is_sse_first_chunk` must detect `event:` and `: ` prefixes

#### `cassette_or_o4mini_tool`
- o4-mini via OpenRouter (same underlying format as direct OpenAI)
- Asserts: OPENAI_CLASSIC present, fake absent

#### `cassette_or_error_no_credits`
- 402 response
- Asserts: proxy returns 402, no cache write

---

### Gemini Tests

#### `cassette_gem_tool_call`
- `functionCall.args.key` contains fake
- Asserts: `GCP` present, fake absent

#### `cassette_gem_multi_part_thinking`
- Thought parts (thought:true) + tool call
- Asserts: `GCP` present, fake absent
- Asserts: `"thought":true` in output (thinking passed through)

#### `cassette_gem_colocated_finish`
```rust
/// Known limitation probe: Gemini co-locates finishReason with content in the final frame.
/// The proxy's restore stream accumulates the content, emits restored synthetic frames,
/// and DROPS finishReason (it's in a path-B frame, not path-A, so it cannot be
/// a classify_terminal target).
///
/// This test DOCUMENTS the known limitation.
/// See: crates/lcp-server/SPEC.md §Known Limitations
///      tests/spec/sse_terminal_ordering.rs::vc_sse_19_gemini_colocated_finish_reason_preserved (#[ignore])
#[tokio::test]
async fn cassette_gem_colocated_finish() {
    let c = Cassette::load("tests/fixtures/gemini/colocated_finish_reason.toml");
    let (harness, _secret, fake_bytes) = cassette_harness(&c).await;
    // ...
    // Restore MUST succeed (content is restored)
    assert_present(&bytes, &[GCP], "gem_colocated: secret must be present");
    assert_absent(&bytes, &[&fake_bytes], "gem_colocated: fake must be absent");
    // finishReason is a known gap — document but don't fail
    let finish_present = bytes.windows(b"\"finishReason\"".len())
        .any(|w| w == b"\"finishReason\"");
    if !finish_present {
        eprintln!(
            "KNOWN LIMITATION: Gemini finishReason dropped when restore stream active. \
             See SPEC.md §Known Limitations."
        );
    }
    // If a future fix lands, this probe will start printing nothing.
}
```

#### `cassette_gem_text_only`
- Plain text reply
- Asserts: `GCP` present, fake absent

#### `cassette_gem_usage_metadata`
- `usageMetadata` in response (NOT co-located with content)
- Asserts: `GCP` present, fake absent
- Asserts: `"usageMetadata"` appears in proxy output
  (usageMetadata should pass through even when restore active)

#### `cassette_gem_error_quota`
- 429 quota exceeded
- Asserts: proxy returns 429, no cache write

---

## Cache Behavior Tests (new, cross-provider)

These use cassettes to verify cache semantics:

#### `cassette_cache_hit_replay`
- Send the same request twice (same cassette queued twice)
- First: `x-lcp-cache: MISS`
- Second: `x-lcp-cache: HIT`
- Asserts: HIT response bytes == MISS response bytes (exact equality)
- Asserts: mock upstream received exactly 1 request (HIT uses cache, not upstream)

#### `cassette_cache_miss_non_stream_json`
- Use a JSON cassette (non-SSE response)
- Asserts: proxy caches the response
- Asserts: no SSE restoration runs (non-stream path)

#### `cassette_error_not_cached`
- Use any error cassette (4xx/5xx)
- Send twice
- Asserts: second response also shows `x-lcp-cache: MISS`
  (errors must not be cached, per SPEC)
- Asserts: mock upstream received 2 requests

---

## Acceptance Criteria

- [ ] `cassettes.rs` created under `tests/integration/`
- [ ] One test per cassette (34 cassettes → 34 tests) plus 3 cache behavior tests
- [ ] All 37 tests pass with `cargo nextest run --test integration`
- [ ] `cassette_or_claude_finish_colocated` fails if `extract_fields` empty-content guard
  is reverted (regression test is effective)
- [ ] `cassette_oai_resp_text_delta` fails if synthetic frame event type is "output_text"
  instead of "response.output_text.delta" (regression test is effective)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] No test uses live network (all mock via cassettes)

## Reviewer Instructions

- Confirm each regression-guard cassette test would fail with the relevant prior bug:
  - `cassette_or_claude_finish_colocated`: fails when `extract_fields` returns `content:""`
  - `cassette_oai_resp_text_delta`: fails when synthetic event type is "output_text"
  - `cassette_ant_tool_use_input_json`: fails when `flush_safe_prefix` doesn't retract at JSON boundaries
- Confirm `cassette_gem_colocated_finish` does NOT fail (it's a known-gap probe, not a hard assert)
- Confirm `cassette_error_not_cached` sends 2 upstream requests
- Confirm `cassette_cache_hit_replay` sends exactly 1 upstream request
