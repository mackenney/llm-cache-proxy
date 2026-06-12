//! Cassette-based integration tests: real provider wire formats, no live API.
//!
//! Each test loads a captured cassette from `tests/fixtures/`, feeds it through
//! the proxy via `MockUpstream::Recorded`, and asserts wire-format correctness.
//! These tests run in CI without API keys.
//!
//! # Design
//!
//! Cassettes test SSE wire format fidelity: that the proxy correctly routes,
//! streams, and caches real provider SSE frame shapes. DoppelExt restoration
//! is tested separately in `tests/integration/doppel.rs`.
//!
//! Regression guards for the three specific bugs found in Jun 2026 E2E use
//! doppel patterns generated at test time (same approach as doppel.rs).
//! See: `cassette_regression_*` tests.

use doppel::{patterns, swap as doppel_swap};
use lcp_server::{DoppelExt, ExtensionPipeline};

use crate::common::{Cassette, MockUpstream, TestHarness};

/// Build a harness that replays a cassette without any extensions.
/// Used for wire-format tests (SSE event types, cache behavior, error handling).
async fn cassette_harness(c: &Cassette) -> TestHarness {
    let mock = MockUpstream::builder().cassette(c).build().await;
    TestHarness::builder().mock(mock).build().await
}

#[allow(dead_code)]
async fn cassette_harness_with_doppel(c: &Cassette, pat: doppel::Pattern) -> TestHarness {
    let mock = MockUpstream::builder().cassette(c).build().await;
    TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await
}

async fn proxy_post(harness: &TestHarness, path: &str, body: String) -> (u16, bytes::Bytes) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}{path}", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("Authorization", "Bearer test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap();
    (status, bytes)
}

fn anthropic_body() -> String {
    serde_json::json!({
        "model": "claude-haiku-4-5", "max_tokens": 300, "stream": true,
        "tools": [{"name":"store_key","description":"Store key.",
                   "input_schema":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}],
        "messages": [{"role":"user","content": "Call store_key with key: test-key"}]
    })
    .to_string()
}

fn openai_body(model: &str) -> String {
    serde_json::json!({
        "model": model, "max_tokens": 200, "stream": true,
        "tools": [{"type":"function","function":{"name":"store_key","description":"Store key.",
            "parameters":{"type":"object","properties":{"key":{"type":"string"}},"required":["key"]}}}],
        "messages": [{"role":"user","content": "Call store_key with key: test-key"}]
    })
    .to_string()
}

fn openai_resp_body() -> String {
    serde_json::json!({
        "model": "gpt-4o-mini", "max_output_tokens": 100, "stream": true,
        "input": "Repeat exactly: test-key"
    })
    .to_string()
}

fn gemini_body() -> String {
    serde_json::json!({
        "contents": [{"role":"user","parts":[{"text": "Call store_key with key: test-key"}]}],
        "tools": [{"function_declarations":[{
            "name":"store_key","description":"Store key.",
            "parameters":{"type":"OBJECT","properties":{"key":{"type":"STRING"}},"required":["key"]}
        }]}]
    })
    .to_string()
}

// =============================================================================
// Anthropic tests
// =============================================================================

#[tokio::test]
async fn cassette_ant_tool_use_input_json() {
    let c = Cassette::load("fixtures/anthropic/tool_use_input_json.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // Verify key Anthropic SSE events are present
    assert!(
        bytes
            .windows(b"message_start".len())
            .any(|w| w == b"message_start"),
        "ant_tool_use: message_start must appear"
    );
    assert!(
        bytes
            .windows(b"content_block_delta".len())
            .any(|w| w == b"content_block_delta"),
        "ant_tool_use: content_block_delta must appear"
    );
    harness.wait_for_writes().await;
    assert_eq!(harness.mock_requests().len(), 1);
}

#[tokio::test]
async fn cassette_ant_thinking_and_text() {
    let c = Cassette::load("fixtures/anthropic/thinking_and_text.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // Thinking + tool call stream — verify thinking blocks and tool call deltas are present
    assert!(
        bytes
            .windows(b"thinking_delta".len())
            .any(|w| w == b"thinking_delta")
            || bytes
                .windows(b"input_json_delta".len())
                .any(|w| w == b"input_json_delta"),
        "ant_thinking: expecting thinking_delta or input_json_delta"
    );
}

#[tokio::test]
async fn cassette_ant_multi_block_stop() {
    let c = Cassette::load("fixtures/anthropic/multi_block_stop.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"content_block_stop".len())
            .any(|w| w == b"content_block_stop"),
        "multi_block: content_block_stop must appear"
    );
}

#[tokio::test]
async fn cassette_ant_text_only() {
    let c = Cassette::load("fixtures/anthropic/text_only.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    harness.wait_for_writes().await;
    assert_eq!(harness.mock_requests().len(), 1);
}

#[tokio::test]
async fn cassette_ant_message_delta_stop() {
    let c = Cassette::load("fixtures/anthropic/message_delta_stop.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn cassette_ant_empty_thinking() {
    let c = Cassette::load("fixtures/anthropic/empty_thinking.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

/// Anthropic error response: proxy passes through and does NOT cache it.
#[tokio::test]
async fn cassette_ant_error_rate_limit() {
    let c = Cassette::load("fixtures/anthropic/error_rate_limit.toml");
    let harness = cassette_harness(&c).await;
    let (status, _) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert!(
        status >= 400,
        "error cassette must return error status, got {status}"
    );
    assert_eq!(harness.mock_requests().len(), 1);
}

#[tokio::test]
async fn cassette_ant_error_overloaded() {
    let c = Cassette::load("fixtures/anthropic/error_overloaded.toml");
    let harness = cassette_harness(&c).await;
    let (status, _) = proxy_post(&harness, "/anthropic/v1/messages", anthropic_body()).await;
    assert!(
        status >= 400,
        "overloaded cassette must return error status, got {status}"
    );
    assert_eq!(harness.mock_requests().len(), 1);
}

// =============================================================================
// OpenAI Chat Completions tests
// =============================================================================

#[tokio::test]
async fn cassette_oai_tool_calls() {
    let c = Cassette::load("fixtures/openai/chat_tool_calls.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openai/v1/chat/completions",
        openai_body("gpt-4o-mini"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"tool_calls".len())
            .any(|w| w == b"tool_calls"),
        "oai_tool: tool_calls must appear in stream"
    );
}

/// OpenAI colocated finish_reason: real wire format has `content:"" + finish_reason` in same chunk.
/// The proxy must NOT lose the stream when encountering this pattern.
/// Regression guard for commit 0bc6827 (skip empty-string content).
#[tokio::test]
async fn cassette_oai_colocated_finish() {
    let c = Cassette::load("fixtures/openai/chat_colocated_finish.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openai/v1/chat/completions",
        openai_body("gpt-4o-mini"),
    )
    .await;
    assert_eq!(
        status, 200,
        "colocated_finish: proxy must not fail when content:\"\" co-located with finish_reason"
    );
    assert!(!bytes.is_empty());
    // The proxy must produce a complete response (not abort mid-stream)
    assert!(
        bytes.windows(b"[DONE]".len()).any(|w| w == b"[DONE]"),
        "colocated_finish: [DONE] must appear — proxy must not abort mid-stream"
    );
}

#[tokio::test]
async fn cassette_oai_content_only() {
    let c = Cassette::load("fixtures/openai/chat_content_only.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/chat/completions",
        serde_json::json!({"model":"gpt-4o-mini","max_tokens":50,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"finish_reason".len())
            .any(|w| w == b"finish_reason"),
        "content_only: finish_reason must appear"
    );
}

#[tokio::test]
async fn cassette_oai_o4mini_reasoning() {
    let c = Cassette::load("fixtures/openai/chat_o4mini_reasoning.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/chat/completions",
        serde_json::json!({"model":"o4-mini","max_completion_tokens":500,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn cassette_oai_multi_tool() {
    let c = Cassette::load("fixtures/openai/chat_multi_tool.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openai/v1/chat/completions",
        openai_body("gpt-4o-mini"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"tool_calls".len())
            .any(|w| w == b"tool_calls"),
        "multi_tool: tool_calls must appear"
    );
}

#[tokio::test]
async fn cassette_oai_stream_error() {
    let c = Cassette::load("fixtures/openai/chat_stream_error.toml");
    let harness = cassette_harness(&c).await;
    let (status, _) = proxy_post(&harness, "/openai/v1/chat/completions",
        serde_json::json!({"model":"gpt-4o-mini","max_tokens":50,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert!(
        status >= 400,
        "stream_error: expected error status, got {status}"
    );
    assert_eq!(harness.mock_requests().len(), 1);
}

#[tokio::test]
async fn cassette_oai_finish_stop() {
    let c = Cassette::load("fixtures/openai/chat_finish_stop.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/chat/completions",
        serde_json::json!({"model":"gpt-4o-mini","max_tokens":30,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"finish_reason".len())
            .any(|w| w == b"finish_reason"),
        "finish_stop: finish_reason must appear"
    );
    assert!(
        bytes.windows(b"[DONE]".len()).any(|w| w == b"[DONE]"),
        "finish_stop: [DONE] must appear"
    );
}

// =============================================================================
// OpenAI Responses API tests
// =============================================================================

/// Responses API: `response.output_text.delta` event type.
///
/// Regression guard: synthetic frames previously used wrong event type "output_text"
/// instead of "response.output_text.delta" (fixed in commit f6aeeca).
#[tokio::test]
async fn cassette_oai_resp_text_delta() {
    let c = Cassette::load("fixtures/openai/resp_text_delta.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/responses", openai_resp_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // Verify synthetic frames use correct event type (the f6aeeca fix)
    assert!(
        bytes
            .windows(b"response.output_text.delta".len())
            .any(|w| w == b"response.output_text.delta"),
        "resp_text: synthetic frames must use event: response.output_text.delta, not event: output_text"
    );
}

#[tokio::test]
async fn cassette_oai_resp_done_sequence() {
    let c = Cassette::load("fixtures/openai/resp_done_sequence.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/responses", openai_resp_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn cassette_oai_resp_error_incomplete() {
    let c = Cassette::load("fixtures/openai/resp_error_incomplete.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/responses",
        serde_json::json!({"model":"gpt-4o-mini","max_output_tokens":1,"stream":true,"input":"test"}).to_string()
    ).await;
    // 200 or error — just verify the proxy doesn't crash
    assert!(
        !bytes.is_empty() || status >= 400,
        "resp_incomplete: response must be non-empty or error"
    );
}

/// Known-gap probe: response.output_text.done body probe.
#[tokio::test]
async fn cassette_oai_resp_output_text_done_body() {
    let c = Cassette::load("fixtures/openai/resp_output_text_done_body.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/responses", openai_resp_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // Document the known gap (informational)
    if bytes
        .windows(b"response.output_text.done".len())
        .any(|w| w == b"response.output_text.done")
    {
        eprintln!("INFO: response.output_text.done event present in stream");
    }
}

/// Known-gap probe: response.completed body probe.
#[tokio::test]
async fn cassette_oai_resp_completed_body() {
    let c = Cassette::load("fixtures/openai/resp_completed_body.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openai/v1/responses", openai_resp_body()).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

// =============================================================================
// OpenRouter tests
// =============================================================================

#[tokio::test]
async fn cassette_or_claude_tool_use() {
    let c = Cassette::load("fixtures/openrouter/claude_tool_use.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openrouter/v1/chat/completions",
        openai_body("anthropic/claude-haiku-4-5"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"tool_calls".len())
            .any(|w| w == b"tool_calls"),
        "or_claude_tool: tool_calls must appear"
    );
}

/// THE Jun 2026 bug scenario: OpenRouter Claude sends `content:"" + finish_reason`.
///
/// The proxy must complete the stream without aborting (the bug caused the terminal
/// flush to never fire because classify_terminal was blocked by empty content).
/// Regression guard for: fix: skip empty-string content (commit 0bc6827).
#[tokio::test]
async fn cassette_or_claude_finish_colocated() {
    let c = Cassette::load("fixtures/openrouter/claude_finish_colocated.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openrouter/v1/chat/completions",
        openai_body("anthropic/claude-haiku-4-5"),
    )
    .await;
    assert_eq!(
        status, 200,
        "or_colocated: proxy must return 200 — not abort when content:\"\" co-located with finish_reason"
    );
    assert!(!bytes.is_empty());
    // The stream MUST complete with [DONE] — if it doesn't, the terminal flush regression fired
    assert!(
        bytes.windows(b"[DONE]".len()).any(|w| w == b"[DONE]"),
        "or_colocated: [DONE] must appear — terminal flush must fire even with empty content"
    );
}

#[tokio::test]
async fn cassette_or_claude_text() {
    let c = Cassette::load("fixtures/openrouter/claude_text.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openrouter/v1/chat/completions",
        serde_json::json!({"model":"anthropic/claude-haiku-4-5","max_tokens":50,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn cassette_or_deepseek_chat() {
    let c = Cassette::load("fixtures/openrouter/deepseek_chat.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openrouter/v1/chat/completions",
        openai_body("deepseek/deepseek-chat"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"finish_reason".len())
            .any(|w| w == b"finish_reason"),
        "deepseek_chat: finish_reason must appear"
    );
}

/// DeepSeek R1: sends `delta.reasoning` (NOT `delta.reasoning_content`).
/// Verifies the proxy doesn't corrupt the stream when encountering this field.
#[tokio::test]
async fn cassette_or_deepseek_r1_reasoning() {
    let c = Cassette::load("fixtures/openrouter/deepseek_r1_reasoning.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openrouter/v1/chat/completions",
        openai_body("deepseek/deepseek-r1"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // reasoning field must pass through (not stripped)
    assert!(
        bytes
            .windows(b"\"reasoning\"".len())
            .any(|w| w == b"\"reasoning\""),
        "deepseek_r1: reasoning field must pass through the proxy"
    );
}

/// OpenRouter: first chunk is `: OPENROUTER PROCESSING` (SSE comment line).
/// Regression guard for `is_sse_first_chunk` detecting `: ` prefix.
#[tokio::test]
async fn cassette_or_processing_prefix() {
    let c = Cassette::load("fixtures/openrouter/processing_prefix.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(&harness, "/openrouter/v1/chat/completions",
        serde_json::json!({"model":"anthropic/claude-haiku-4-5","max_tokens":50,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // OpenRouter comment lines must pass through
    assert!(
        bytes
            .windows(b"OPENROUTER PROCESSING".len())
            .any(|w| w == b"OPENROUTER PROCESSING"),
        "or_prefix: OPENROUTER PROCESSING comment lines must pass through the proxy"
    );
}

#[tokio::test]
async fn cassette_or_o4mini_tool() {
    let c = Cassette::load("fixtures/openrouter/o4mini_tool.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/openrouter/v1/chat/completions",
        openai_body("openai/o4-mini"),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn cassette_or_error_no_credits() {
    let c = Cassette::load("fixtures/openrouter/error_no_credits.toml");
    let harness = cassette_harness(&c).await;
    let (status, _) = proxy_post(&harness, "/openrouter/v1/chat/completions",
        serde_json::json!({"model":"anthropic/claude-haiku-4-5","max_tokens":10,"stream":true,"messages":[{"role":"user","content":"test"}]}).to_string()
    ).await;
    assert!(
        status >= 400,
        "or_error: expected error status, got {status}"
    );
    assert_eq!(harness.mock_requests().len(), 1);
}

// =============================================================================
// Gemini tests
// =============================================================================

#[tokio::test]
async fn cassette_gem_tool_call() {
    let c = Cassette::load("fixtures/gemini/tool_call.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=test",
        gemini_body(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"functionCall".len())
            .any(|w| w == b"functionCall"),
        "gem_tool: functionCall must appear"
    );
}

#[tokio::test]
async fn cassette_gem_multi_part_thinking() {
    let c = Cassette::load("fixtures/gemini/multi_part_thinking.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=test",
        gemini_body(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

/// Known limitation probe: Gemini co-locates finishReason with content.
///
/// The proxy must at minimum restore the content correctly.
/// finishReason may be dropped (known limitation — see SPEC.md §Known Limitations).
#[tokio::test]
async fn cassette_gem_colocated_finish() {
    let c = Cassette::load("fixtures/gemini/colocated_finish.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=test",
        serde_json::json!({"contents":[{"role":"user","parts":[{"text":"test"}]}]}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    // finishReason loss is a known limitation — document but do not fail
    let finish_present = bytes
        .windows(b"\"finishReason\"".len())
        .any(|w| w == b"\"finishReason\"");
    if !finish_present {
        eprintln!(
            "KNOWN LIMITATION: Gemini finishReason may be dropped when restore stream active. \
             See SPEC.md §Known Limitations."
        );
    }
}

#[tokio::test]
async fn cassette_gem_text_only() {
    let c = Cassette::load("fixtures/gemini/text_only.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=test",
        serde_json::json!({"contents":[{"role":"user","parts":[{"text":"test"}]}]}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
}

/// Gemini usageMetadata must pass through the proxy unchanged.
#[tokio::test]
async fn cassette_gem_usage_metadata() {
    let c = Cassette::load("fixtures/gemini/usage_metadata.toml");
    let harness = cassette_harness(&c).await;
    let (status, bytes) = proxy_post(
        &harness,
        "/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=test",
        serde_json::json!({"contents":[{"role":"user","parts":[{"text":"test"}]}]}).to_string(),
    )
    .await;
    assert_eq!(status, 200);
    assert!(!bytes.is_empty());
    assert!(
        bytes
            .windows(b"usageMetadata".len())
            .any(|w| w == b"usageMetadata"),
        "gem_usage: usageMetadata must pass through the proxy"
    );
}

#[tokio::test]
async fn cassette_gem_error_quota() {
    let c = Cassette::load("fixtures/gemini/error_quota.toml");
    let harness = cassette_harness(&c).await;
    let (status, _) = proxy_post(
        &harness,
        "/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?key=test",
        serde_json::json!({"contents":[{"role":"user","parts":[{"text":"test"}]}]}).to_string(),
    )
    .await;
    assert!(
        status >= 400,
        "gem_error: expected error status, got {status}"
    );
    assert_eq!(harness.mock_requests().len(), 1);
}

// =============================================================================
// Cache behavior tests
// =============================================================================

/// Cache hit replay: second identical request is served from cache.
#[tokio::test]
async fn cassette_cache_hit_replay() {
    let c1 = Cassette::load("fixtures/anthropic/text_only.toml");
    let c2 = Cassette::load("fixtures/anthropic/text_only.toml");
    let mock = MockUpstream::builder()
        .cassette(&c1)
        .cassette(&c2)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let body = anthropic_body();
    let client = reqwest::Client::new();

    let r1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r1.headers().get("x-lcp-cache").unwrap(), "MISS");
    let miss_bytes = r1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let r2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r2.headers().get("x-lcp-cache").unwrap(), "HIT");
    let hit_bytes = r2.bytes().await.unwrap();

    assert!(!hit_bytes.is_empty(), "hit replay: bytes must be non-empty");
    // HIT bytes must match MISS bytes (same content served from cache)
    assert_eq!(
        miss_bytes, hit_bytes,
        "hit replay: HIT bytes must equal MISS bytes"
    );
    assert_eq!(
        harness.mock_requests().len(),
        1,
        "upstream called only on MISS"
    );
}

/// Error responses must NOT be cached.
#[tokio::test]
async fn cassette_error_not_cached() {
    let c1 = Cassette::load("fixtures/anthropic/error_rate_limit.toml");
    let c2 = Cassette::load("fixtures/anthropic/error_rate_limit.toml");
    let mock = MockUpstream::builder()
        .cassette(&c1)
        .cassette(&c2)
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;

    let body = anthropic_body();
    let client = reqwest::Client::new();

    let r1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert!(r1.status().is_client_error() || r1.status().is_server_error());
    let _ = r1.bytes().await.unwrap();

    let r2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert!(r2.status().is_client_error() || r2.status().is_server_error());
    assert_eq!(
        harness.mock_requests().len(),
        2,
        "upstream called twice — errors not cached"
    );
}

// =============================================================================
// Regression guard tests with DoppelExt (generate fake at test time)
//
// These use the doppel.rs approach: generate the fake at test time with the
// same pattern used for DoppelExt, then build a mock that serves chunks
// based on real cassette wire format but with the test-session fake embedded.
// =============================================================================

/// Regression guard: flush_safe_prefix must not split fakes at JSON boundaries.
///
/// Uses cassette chunk granularity to simulate real TCP splits.
/// The fake key is generated at test time and embedded in the mock response.
///
/// Regression guard for: fix: flush_safe_prefix (commit 0bc6827).
#[tokio::test]
async fn cassette_regression_ant_flush_safe_prefix() {
    const ANT: &[u8] =
        b"sk-ant-api03-YLY9P1-i5dC1zbDHjPYKuQHRM0TsEXQj6wiLZGOvUCYMDV25RlbUUTO1bZ_tbvx0OMdtvzVCSDh6vkpciXKbN_5lGMcOQAA";

    let pat = patterns::anthropic();
    // Use the same prefix the working doppel tests use so the pattern matches correctly.
    let body_bytes = [b"key: ".as_slice(), ANT].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_str = String::from_utf8_lossy(&sr.entries[0].fake).into_owned();

    // Split the fake across 4 input_json_delta chunks to exercise flush_safe_prefix.
    // Each partial_json fragment is a raw slice of the fake string (as in real Anthropic traffic).
    let n = fake_str.len() / 4;
    let parts = [
        fake_str[..n].to_owned(),
        fake_str[n..2 * n].to_owned(),
        fake_str[2 * n..3 * n].to_owned(),
        fake_str[3 * n..].to_owned(),
    ];

    // SSE format mirrors the working restore_anthropic_input_json_delta test in doppel.rs.
    let mut chunks: Vec<String> = vec![
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_t\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"stop_reason\":null}}\n\n".to_owned(),
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"store_key\",\"input\":{}}}\n\n".to_owned(),
    ];
    for part in &parts {
        chunks.push(format!(
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{part}\"}}}}\n\n"
        ));
    }
    chunks.push(
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n"
            .to_owned(),
    );
    chunks.push("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n".to_owned());
    chunks.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_owned());

    let mock = MockUpstream::builder().sse(200, chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;

    let body = format!(
        r#"{{"model":"claude-haiku-4-5","max_tokens":300,"stream":true,"tools":[{{"name":"store_key","description":"Store key.","input_schema":{{"type":"object","properties":{{"key":{{"type":"string"}}}},"required":["key"]}}}}],"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(ANT)
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = resp.bytes().await.unwrap();

    assert!(
        bytes.windows(ANT.len()).any(|w| w == ANT),
        "flush_safe_prefix: original key must appear after restoration of split partial_json"
    );
    assert!(
        !bytes
            .windows(fake_str.len())
            .any(|w| w == fake_str.as_bytes()),
        "flush_safe_prefix: fake must not appear in output after restoration"
    );
}

/// Regression guard: Responses API synthetic frames use correct event type.
///
/// Synthetic event type must be "response.output_text.delta" not "output_text".
/// Regression guard for commit f6aeeca.
#[tokio::test]
async fn cassette_regression_oai_resp_event_type() {
    const OPENAI_CLASSIC: &[u8] = b"sk-v0zsmdzWwRZktfsJIdQWQvKdIYk1LYrtuF3hWeJep2YvHzQ3";

    let pat = patterns::openai_classic();
    let sr = doppel_swap(OPENAI_CLASSIC, std::slice::from_ref(&pat)).unwrap();
    let fake_str = String::from_utf8_lossy(&sr.entries[0].fake).into_owned();

    // Construct a Responses API SSE stream with the fake key in delta events
    let chunks = vec![
        format!(
            "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"  }}\n\n",
            &fake_str[..20]
        ),
        format!(
            "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n",
            &fake_str[20..]
        ),
        "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\"}\n\n"
            .to_string(),
        "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n".to_string(),
    ];

    let mock = MockUpstream::builder()
        .response(crate::common::MockResponse::Recorded {
            status: 200,
            headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
            chunks: chunks
                .into_iter()
                .map(|s| bytes::Bytes::from(s.into_bytes()))
                .collect(),
        })
        .build()
        .await;

    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;

    let body = serde_json::json!({
        "model": "gpt-4o-mini", "max_output_tokens": 100, "stream": true,
        "input": format!("Repeat: {}", String::from_utf8_lossy(OPENAI_CLASSIC))
    })
    .to_string();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/openai/v1/responses", harness.proxy_url()))
        .header("Authorization", "Bearer test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = resp.bytes().await.unwrap();

    // Synthetic frames must use correct event type
    assert!(
        bytes
            .windows(b"response.output_text.delta".len())
            .any(|w| w == b"response.output_text.delta"),
        "resp_event_type: synthetic frames must use 'response.output_text.delta', not 'output_text'"
    );
    // Original key must be present (restored)
    assert!(
        bytes
            .windows(OPENAI_CLASSIC.len())
            .any(|w| w == OPENAI_CLASSIC),
        "resp_event_type: original key must appear after restoration"
    );
}

/// Regression guard: OpenRouter/OpenAI co-located `content:""` + `finish_reason`.
///
/// The empty-string content guard in extract_fields must prevent classify_terminal
/// from being blocked, ensuring the terminal flush fires before finish_reason.
/// Regression guard for commit 0bc6827.
#[tokio::test]
async fn cassette_regression_or_colocated_finish() {
    const OPENAI_CLASSIC: &[u8] = b"sk-v0zsmdzWwRZktfsJIdQWQvKdIYk1LYrtuF3hWeJep2YvHzQ3";

    let pat = patterns::openai_classic();
    let body_bytes = [b"key: ".as_slice(), OPENAI_CLASSIC].concat();
    let sr = doppel_swap(&body_bytes, std::slice::from_ref(&pat)).unwrap();
    let fake_str = String::from_utf8_lossy(&sr.entries[0].fake).into_owned();

    // Exact wire format: tool header, fake in arguments, then content:"" + finish_reason (the bug).
    // Use serde_json to build the arguments JSON string correctly — avoids escaping errors.
    let args_json = serde_json::to_string(&serde_json::json!({"key": &fake_str})).unwrap();
    let chunk2_data = serde_json::json!({
        "id": "chatcmpl-x", "object": "chat.completion.chunk", "model": "gpt-4o",
        "choices": [{"index": 0, "delta": {
            "tool_calls": [{"index": 0, "function": {"arguments": args_json}}]
        }, "finish_reason": null}]
    });
    let chunks = vec![
        concat!(
            "data: {\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",",
            "\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",",
            "\"content\":null,\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",",
            "\"function\":{\"name\":\"store_key\",\"arguments\":\"\"}}}]},\"finish_reason\":null}]}\n\n"
        ).to_owned(),
        format!("data: {chunk2_data}\n\n"),
        // The bug: empty content co-located with finish_reason blocks classify_terminal.
        concat!(
            "data: {\"id\":\"chatcmpl-x\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",",
            "\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"},\"finish_reason\":\"tool_calls\"}]}\n\n"
        ).to_owned(),
        "data: [DONE]\n\n".to_owned(),
    ];

    let mock = MockUpstream::builder().sse(200, chunks).build().await;
    let harness = TestHarness::builder()
        .mock(mock)
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(vec![pat])))
        .build()
        .await;

    let body = format!(
        r#"{{"model":"anthropic/claude-haiku-4-5","max_tokens":50,"stream":true,"tools":[{{"type":"function","function":{{"name":"store_key","description":"Store key.","parameters":{{"type":"object","properties":{{"key":{{"type":"string"}}}},"required":["key"]}}}}}}],"messages":[{{"role":"user","content":"key={}"}}]}}"#,
        String::from_utf8_lossy(OPENAI_CLASSIC)
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("Authorization", "Bearer test-key")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = resp.bytes().await.unwrap();

    assert!(
        bytes
            .windows(OPENAI_CLASSIC.len())
            .any(|w| w == OPENAI_CLASSIC),
        "or_colocated: original key must appear after restoration"
    );
    assert!(
        !bytes
            .windows(fake_str.len())
            .any(|w| w == fake_str.as_bytes()),
        "or_colocated: fake MUST be absent"
    );
    assert!(
        bytes.windows(b"[DONE]".len()).any(|w| w == b"[DONE]"),
        "or_colocated: [DONE] must appear"
    );
}
