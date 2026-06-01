# Step 04: Multi-Provider SSE Integration Tests

## Context

### Overall Objective

Implement SSE-aware unscrubbing so that fake keys distributed token-by-token across
Anthropic/OpenAI/Gemini SSE `data:` events are detected at the text level and replaced
before the response reaches the client or cache.

### Phase Context

Step 03 wired `SseUnscrubStream` and made the Anthropic test pass. This step adds the
same coverage for the remaining providers (OpenAI/OpenRouter share a format; Gemini differs).
Tests must mirror the pattern of `unscrub_restores_secret_from_anthropic_sse_stream`.

### This Step

Add two new integration tests to `tests/integration/scrub.rs`:

1. `unscrub_restores_secret_from_openai_sse_stream` — splits a fake OpenAI classic key
   across `choices[0].delta.content` events.
2. `unscrub_restores_secret_from_gemini_sse_stream` — splits a fake GCP API key across
   `candidates[0].content.parts[0].text` events.

OpenRouter uses the identical SSE event format as OpenAI; a single test covering OpenAI
covers both.

## Prerequisites

- Step 03 merged (SSE-aware unscrubbing working end-to-end for Anthropic).

## Files to Read Before Starting

- `tests/integration/scrub.rs` — read lines 636–727 (existing Anthropic SSE test) very
  carefully; new tests must follow the exact same structure
- `tests/common/mock_upstream.rs` — `MockUpstream::builder().sse(200, chunks)` usage
- `crates/lcp-server/SPEC.md §SSE-Aware Unscrubbing` — provider SSE formats

## Implementation

### Task 1: Add `unscrub_restores_secret_from_openai_sse_stream`

Place after the existing Anthropic SSE test (at the bottom of `tests/integration/scrub.rs`).

**SSE event format for OpenAI**:
```
data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"content":"<text>"},"finish_reason":null}]}\n\n
```

**Test structure** (mirror the Anthropic test exactly, adapted for OpenAI):

```rust
#[tokio::test]
async fn unscrub_restores_secret_from_openai_sse_stream() {
    use its_classified::scrub as ic_scrub;

    let pat = patterns::openai_classic();
    let body_bytes = [b"key: ".as_slice(), OPENAI_CLASSIC].concat();
    let sr = ic_scrub(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 4 delta events.
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = Vec::new();
    for part in &parts {
        sse_chunks.push(format!(
            "data: {{\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{part}\"}},\"finish_reason\":null}}]}}\n\n"
        ));
    }
    sse_chunks.push("data: {\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n".to_owned());
    sse_chunks.push("data: [DONE]\n\n".to_owned());

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(ScrubExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"model":"gpt-4o","max_tokens":200,"stream":true,"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(OPENAI_CLASSIC)
    );

    let resp = client
        .post(format!("{}/openai/v1/chat/completions", harness.proxy_url()))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp_bytes = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert_present(
        &resp_bytes,
        &[OPENAI_CLASSIC],
        "client OpenAI SSE response: Phase 3 must restore original secret",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client OpenAI SSE response: scrubbed fake must not be visible",
    );
}
```

### Task 2: Add `unscrub_restores_secret_from_gemini_sse_stream`

**SSE event format for Gemini** (streaming `generateContent`):
```
data: {"candidates":[{"content":{"parts":[{"text":"<text>"}],"role":"model"},"finishReason":"STOP","index":0}]}\n\n
```

Note: Gemini streaming responses do NOT have a `[DONE]` terminator — the stream simply ends
after the last event.

```rust
#[tokio::test]
async fn unscrub_restores_secret_from_gemini_sse_stream() {
    use its_classified::scrub as ic_scrub;

    let pat = patterns::gcp();
    let body_bytes = [b"key: ".as_slice(), GCP].concat();
    let sr = ic_scrub(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_bytes = sr.entries[0].fake.clone();
    let fake_str = String::from_utf8_lossy(&fake_bytes).into_owned();

    // Split the fake across 4 delta events.
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    let mut sse_chunks: Vec<String> = Vec::new();
    for part in &parts {
        sse_chunks.push(format!(
            "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"{part}\"}}],\"role\":\"model\"}},\"finishReason\":\"STOP\",\"index\":0}}]}}\n\n"
        ));
    }

    let mock = MockUpstream::builder().sse(200, sse_chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(ScrubExt::new(vec![pat])))
        .build()
        .await;
    let client = reqwest::Client::new();

    let body = format!(
        r#"{{"contents":[{{"parts":[{{"text":"key={}"}}]}}]}}"#,
        String::from_utf8_lossy(GCP)
    );

    let resp = client
        .post(format!(
            "{}/gemini/v1/models/gemini-2.5-flash:streamGenerateContent",
            harness.proxy_url()
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let resp_bytes = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    assert_present(
        &resp_bytes,
        &[GCP],
        "client Gemini SSE response: Phase 3 must restore original secret",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "client Gemini SSE response: scrubbed fake must not be visible",
    );
}
```

### Notes

- `GCP` constant (`b"AIzaSyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"`) is already defined at the top of
  `tests/integration/scrub.rs`. Do NOT redefine it.
- `OPENAI_CLASSIC` is also already defined.
- `assert_present` and `assert_absent` helpers are already defined in the same file.
- `MockUpstream`, `TestHarness`, `ExtensionPipeline`, `ScrubExt` are already imported via
  the `use` statements at the top of the file. Do NOT add duplicate imports.
- The proxy path for Gemini must match the router's expected pattern
  `/<provider>/...` → the test uses `/gemini/v1/models/gemini-2.5-flash:streamGenerateContent`.

## Acceptance Criteria

- [ ] `cargo nextest run --test integration -- scrub::unscrub_restores_secret_from_openai_sse_stream` exits 0
- [ ] `cargo nextest run --test integration -- scrub::unscrub_restores_secret_from_gemini_sse_stream` exits 0
- [ ] `cargo nextest run --test integration` exits 0 (no regressions)
- [ ] `cargo nextest run` exits 0 (all tests pass)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0

## Reviewer Instructions

```bash
cd /home/ignacio/pr/llm-cache-proxy

# New tests must pass
cargo nextest run --test integration -- \
    scrub::unscrub_restores_secret_from_openai_sse_stream \
    scrub::unscrub_restores_secret_from_gemini_sse_stream \
    2>&1 | tail -10

# All integration tests must pass
cargo nextest run --test integration 2>&1 | grep -E 'FAILED|^test result'

# Full suite
cargo nextest run 2>&1 | grep -E 'FAILED|^test result'

# Clippy
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep error | head -5
```

Expected: both new tests PASSED, no FAILED lines in integration or full suite, clippy clean.

## Rollback

`git checkout -- tests/integration/scrub.rs`
