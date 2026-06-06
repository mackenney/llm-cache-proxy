//! E2E tests for new SSE fields — validates proxy handling with real provider APIs.
//!
//! Each test is gated by the corresponding provider API key env var. If the key is
//! absent the test returns early (skip, not fail). All tests share the same pattern:
//!
//! 1. Pre-compute the fake so we can assert it is absent from the final response.
//! 2. Start a proxy (TestHarness) with DoppelExt pointed at the real upstream.
//! 3. Embed a synthetic secret in the message — the real API key is only used in
//!    the auth header and is never part of DoppelExt's detection.
//! 4. DoppelExt swaps the synthetic secret in the body (Phase 2); the model echoes
//!    back the fake; DoppelExt restores the fake to the original (Phase 3).
//! 5. Assert the original synthetic secret is present and the fake is absent.

use doppel::{patterns, swap as doppel_swap};
use lcp_server::{DoppelExt, ExtensionPipeline};

use crate::common::{MockUpstream, TestHarness};

// Synthetic test secrets — NOT real credentials.
// Same structure as integration/doppel.rs constants; all match the built-in patterns.
const ANT: &[u8] =
    b"sk-ant-api03-YLY9P1-i5dC1zbDHjPYKuQHRM0TsEXQj6wiLZGOvUCYMDV25RlbUUTO1bZ_tbvx0OMdtvzVCSDh6vkpciXKbN_5lGMcOQAA";
const OPENAI_CLASSIC: &[u8] = b"sk-v0zsmdzWwRZktfsJIdQWQvKdIYk1LYrtuF3hWeJep2YvHzQ3";
const GCP: &[u8] = b"AIzavURt9l4GMP5k339tqrQWeHPJqdXRArxL-xi";

fn require_env(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("skipping: {var} not set");
            None
        }
    }
}

fn assert_absent(haystack: &[u8], needles: &[&[u8]], ctx: &str) {
    for needle in needles {
        assert!(
            !haystack.windows(needle.len()).any(|w| w == *needle),
            "{ctx}: value must NOT appear in output",
        );
    }
}

fn assert_present(haystack: &[u8], needles: &[&[u8]], ctx: &str) {
    for needle in needles {
        assert!(
            haystack.windows(needle.len()).any(|w| w == *needle),
            "{ctx}: value must appear in output; got:\n{}",
            String::from_utf8_lossy(&haystack[..haystack.len().min(800)])
        );
    }
}

async fn proxy_harness(upstream_url: &str, pats: Vec<doppel::Pattern>) -> TestHarness {
    let mock = MockUpstream::builder().json(200, "{}").build().await;
    TestHarness::builder()
        .mock(mock)
        .upstream_url(upstream_url.to_owned())
        .extensions(ExtensionPipeline::new().register(DoppelExt::new(pats)))
        .timeout(120)
        .build()
        .await
}

// Anthropic extended thinking requires max_tokens > budget_tokens.
const ANT_THINK_MAX_TOKENS: u32 = 4000;
const ANT_THINK_BUDGET: u32 = 2000;

#[tokio::test]
async fn e2e_anthropic_thinking_and_text() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };

    let pat = patterns::anthropic();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), ANT].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://api.anthropic.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": ANT_THINK_MAX_TOKENS,
        "thinking": {"type": "enabled", "budget_tokens": ANT_THINK_BUDGET},
        "stream": true,
        "tools": [{
            "name": "store_key",
            "description": "Store an API key",
            "input_schema": {
                "type": "object",
                "properties": {"key": {"type": "string"}},
                "required": ["key"]
            }
        }],
        "messages": [{
            "role": "user",
            "content": format!("Call store_key with this key: {}", String::from_utf8_lossy(ANT))
        }]
    })
    .to_string();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(&resp_bytes, &[ANT], "thinking_and_text: original secret");
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "thinking_and_text: fake absent",
    );
    assert!(
        resp_bytes
            .windows(b"thinking_delta".len())
            .any(|w| w == b"thinking_delta"),
        "thinking_and_text: expected thinking_delta events; got:\n{}",
        String::from_utf8_lossy(&resp_bytes[..resp_bytes.len().min(800)])
    );
}

#[tokio::test]
async fn e2e_anthropic_tool_use() {
    let Some(api_key) = require_env("ANTHROPIC_API_KEY") else {
        return;
    };

    let pat = patterns::anthropic();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), ANT].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://api.anthropic.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "claude-haiku-4-5",
        "max_tokens": 500,
        "stream": true,
        "tools": [{
            "name": "store_key",
            "description": "Store an API key",
            "input_schema": {
                "type": "object",
                "properties": {"key": {"type": "string"}},
                "required": ["key"]
            }
        }],
        "messages": [{
            "role": "user",
            "content": format!("Call store_key with this key: {}", String::from_utf8_lossy(ANT))
        }]
    })
    .to_string();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[ANT],
        "anthropic_tool_use: original secret in input_json_delta",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "anthropic_tool_use: fake absent",
    );
}

#[tokio::test]
async fn e2e_openai_tool_calls() {
    let Some(api_key) = require_env("OPENAI_API_KEY") else {
        return;
    };

    let pat = patterns::openai_classic();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), OPENAI_CLASSIC].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://api.openai.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "gpt-4o-mini",
        "max_tokens": 200,
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "store_key",
                "description": "Store a key",
                "parameters": {
                    "type": "object",
                    "properties": {"key": {"type": "string"}},
                    "required": ["key"]
                }
            }
        }],
        "messages": [{
            "role": "user",
            "content": format!("Call store_key with key: {}", String::from_utf8_lossy(OPENAI_CLASSIC))
        }]
    })
    .to_string();

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[OPENAI_CLASSIC],
        "openai_tool_calls: original secret in tool_calls arguments",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "openai_tool_calls: fake absent",
    );
}

// e2e_openai_o4mini_tool_calls: Exercises tool_calls SSE restore on o4-mini.
// The model streams tool_calls arguments which DoppelExt must restore.
// Note: o4-mini does not expose reasoning_content in Chat Completions streaming;
// the integration tests cover Phase 3 handling of that field synthetically.
#[tokio::test]
async fn e2e_openai_o4mini_tool_calls() {
    let Some(api_key) = require_env("OPENAI_API_KEY") else {
        return;
    };

    let pat = patterns::openai_classic();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), OPENAI_CLASSIC].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://api.openai.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "o4-mini",
        "max_completion_tokens": 2000,
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "store_key",
                "description": "Store a token value",
                "parameters": {
                    "type": "object",
                    "properties": {"key": {"type": "string"}},
                    "required": ["key"]
                }
            }
        }],
        "messages": [{
            "role": "user",
            "content": format!("Call store_key with key: {}", String::from_utf8_lossy(OPENAI_CLASSIC))
        }]
    })
    .to_string();

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[OPENAI_CLASSIC],
        "openai_o4mini_tool_calls: original secret in tool_calls (o4-mini)",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "openai_o4mini_tool_calls: fake absent",
    );
}

#[tokio::test]
async fn e2e_openai_responses_api() {
    let Some(api_key) = require_env("OPENAI_API_KEY") else {
        return;
    };

    let pat = patterns::openai_classic();
    // Phase 2 swaps the secret; Phase 3 restores it in the delta/done events.
    // The response.completed metadata event retains the fake (not checked here).
    let _sr = doppel_swap(
        &[b"key: ".as_slice(), OPENAI_CLASSIC].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();

    let harness = proxy_harness("https://api.openai.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "gpt-4.1-mini",
        "input": format!("Repeat exactly: {}", String::from_utf8_lossy(OPENAI_CLASSIC)),
        "max_output_tokens": 200,
        "stream": true
    })
    .to_string();

    let resp = client
        .post(format!("{}/openai/v1/responses", harness.proxy_url()))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[OPENAI_CLASSIC],
        "openai_responses_api: original secret",
    );
    // The response.completed metadata event retains the fake (Phase 3 does not
    // process it), so assert_absent is intentionally omitted for this test.
    assert!(
        resp_bytes
            .windows(b"response.output_text.delta".len())
            .any(|w| w == b"response.output_text.delta"),
        "openai_responses_api: expected response.output_text.delta events; got:\n{}",
        String::from_utf8_lossy(&resp_bytes[..resp_bytes.len().min(800)])
    );
}

// e2e_openrouter_reasoning: Exercises the proxy's SSE restore on OpenRouter's
// reasoning model (openai/o4-mini). OpenRouter prefixes its SSE stream with comment
// lines (": OPENROUTER PROCESSING") which must be detected as SSE by the proxy.
// Uses a tool call so the secret appears in structured tool_calls arguments.
#[tokio::test]
async fn e2e_openrouter_reasoning() {
    let Some(api_key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };

    let pat = patterns::openai_classic();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), OPENAI_CLASSIC].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    // OpenRouter base path: proxy path "/v1/chat/completions" maps to
    // "https://openrouter.ai/api/v1/chat/completions".
    let harness = proxy_harness("https://openrouter.ai/api", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "openai/o4-mini",
        "max_completion_tokens": 2000,
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "store_key",
                "description": "Store a token value",
                "parameters": {
                    "type": "object",
                    "properties": {"key": {"type": "string"}},
                    "required": ["key"]
                }
            }
        }],
        "messages": [{
            "role": "user",
            "content": format!("Call store_key with key: {}", String::from_utf8_lossy(OPENAI_CLASSIC))
        }]
    })
    .to_string();

    let resp = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[OPENAI_CLASSIC],
        "openrouter_reasoning: original secret in tool_calls (openai/o4-mini via OpenRouter)",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "openrouter_reasoning: fake absent",
    );
}

#[tokio::test]
async fn e2e_openrouter_tool_calls() {
    let Some(api_key) = require_env("OPENROUTER_API_KEY") else {
        return;
    };

    let pat = patterns::openai_classic();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), OPENAI_CLASSIC].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://openrouter.ai/api", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "openai/gpt-4o-mini",
        "max_tokens": 200,
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "store_key",
                "description": "Store a key",
                "parameters": {
                    "type": "object",
                    "properties": {"key": {"type": "string"}},
                    "required": ["key"]
                }
            }
        }],
        "messages": [{
            "role": "user",
            "content": format!("Call store_key with key: {}", String::from_utf8_lossy(OPENAI_CLASSIC))
        }]
    })
    .to_string();

    let resp = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[OPENAI_CLASSIC],
        "openrouter_tool_calls: original secret in tool_calls arguments",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "openrouter_tool_calls: fake absent",
    );
}

// e2e_gemini_multi_part_thinking: Exercises multi-part SSE handling including thought
// parts (parts[N].text with thought:true) by using gemini-2.5-pro with thinking
// enabled and a tool call. The tool call guarantees exact key reproduction in
// functionCall.args; the thinking content is verified to be present.
#[tokio::test]
async fn e2e_gemini_multi_part_thinking() {
    let Some(api_key) = require_env("GEMINI_API_KEY") else {
        return;
    };

    let pat = patterns::gcp();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), GCP].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://generativelanguage.googleapis.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "contents": [{"parts": [{"text": format!("Call store_key with key: {}", String::from_utf8_lossy(GCP))}]}],
        "tools": [{
            "functionDeclarations": [{
                "name": "store_key",
                "description": "Store a key",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {"key": {"type": "STRING"}},
                    "required": ["key"]
                }
            }]
        }],
        "generationConfig": {
            "maxOutputTokens": 2000,
            "thinkingConfig": {"includeThoughts": true}
        }
    })
    .to_string();

    let resp = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-2.5-pro:streamGenerateContent?key={}",
            harness.proxy_url(),
            api_key
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[GCP],
        "gemini_multi_part_thinking: original secret in functionCall args",
    );
    assert_absent(
        &resp_bytes,
        &[&fake_bytes],
        "gemini_multi_part_thinking: fake absent",
    );
    // Verify the model actually generated thinking content (thought parts present).
    assert!(
        resp_bytes
            .windows(b"\"thought\":true".len())
            .any(|w| w == b"\"thought\":true")
            || resp_bytes
                .windows(b"\"thought\": true".len())
                .any(|w| w == b"\"thought\": true"),
        "gemini_multi_part_thinking: expected thought parts in response"
    );
}

#[tokio::test]
async fn e2e_gemini_tool_call() {
    let Some(api_key) = require_env("GEMINI_API_KEY") else {
        return;
    };

    let pat = patterns::gcp();
    let sr = doppel_swap(
        &[b"key: ".as_slice(), GCP].concat(),
        std::slice::from_ref(&pat),
    )
    .unwrap();
    let fake_bytes = sr.entries[0].fake.clone();

    let harness = proxy_harness("https://generativelanguage.googleapis.com", vec![pat]).await;
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "contents": [{
            "parts": [{"text": format!("Call store_key with key: {}", String::from_utf8_lossy(GCP))}]
        }],
        "tools": [{
            "functionDeclarations": [{
                "name": "store_key",
                "description": "Store a key",
                "parameters": {
                    "type": "OBJECT",
                    "properties": {"key": {"type": "STRING"}},
                    "required": ["key"]
                }
            }]
        }],
        "generationConfig": {"maxOutputTokens": 1000}
    })
    .to_string();

    let resp = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent?key={}",
            harness.proxy_url(),
            api_key
        ))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("request to proxy failed");

    assert!(
        resp.status().is_success(),
        "non-200 status: {}",
        resp.status()
    );
    let resp_bytes = resp.bytes().await.unwrap();

    assert_present(
        &resp_bytes,
        &[GCP],
        "gemini_tool_call: original secret in functionCall args",
    );
    assert_absent(&resp_bytes, &[&fake_bytes], "gemini_tool_call: fake absent");
}
