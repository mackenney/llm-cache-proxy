//! Model extraction tests — spec contract for per-provider model identification.
//!
//! Verifies that the proxy extracts model identifiers correctly for each provider
//! and stores them in cache metadata (entries table and by_model stats).

use crate::common::{MockUpstream, TestHarness};

fn gemini_sse_chunks() -> Vec<&'static str> {
    vec![
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n",
    ]
}

fn gemini_request_body() -> &'static str {
    r#"{"contents":[{"parts":[{"text":"Say hello"}]}]}"#
}

async fn gemini_harness() -> TestHarness {
    let mock = MockUpstream::builder()
        .sse(200, gemini_sse_chunks())
        .build()
        .await;
    TestHarness::builder().mock(mock).build().await
}

#[tokio::test]
async fn test_gemini_model_extracted_from_path() {
    let harness = gemini_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-2.5-flash:generateContent",
            harness.proxy_url()
        ))
        .query(&[("key", "test-api-key")])
        .header("content-type", "application/json")
        .body(gemini_request_body())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].model.as_deref(),
        Some("gemini-2.5-flash"),
        "model must be extracted from Gemini URL path; got: {:?}",
        entries[0].model
    );
}

#[tokio::test]
async fn test_gemini_model_appears_in_by_model_stats() {
    let harness = gemini_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-2.0-flash:generateContent",
            harness.proxy_url()
        ))
        .query(&[("key", "test-api-key")])
        .header("content-type", "application/json")
        .body(gemini_request_body())
        .send()
        .await
        .unwrap();

    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let stats = harness.cache().stats().unwrap();
    assert!(
        stats.by_model.contains_key("gemini-2.0-flash"),
        "by_model must contain Gemini model name from path; got: {:?}",
        stats.by_model
    );
}

#[tokio::test]
async fn test_gemini_stream_generate_content_model_extracted() {
    let harness = gemini_harness().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-1.5-pro:streamGenerateContent",
            harness.proxy_url()
        ))
        .query(&[("key", "test-api-key")])
        .header("content-type", "application/json")
        .body(gemini_request_body())
        .send()
        .await
        .unwrap();

    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "test_gemini_stream_generate_content_model_extracted: expected one cache entry"
    );
    assert_eq!(
        entries[0].model.as_deref(),
        Some("gemini-1.5-pro"),
        "streamGenerateContent verb must not appear in extracted model name"
    );
}

fn anthropic_sse_chunks() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn anthropic_request_body() -> &'static str {
    r#"{"model":"claude-sonnet-4-20250514","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}"#
}

#[tokio::test]
async fn test_anthropic_model_extracted_from_body() {
    let mock = MockUpstream::builder()
        .sse(200, anthropic_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body(anthropic_request_body())
        .send()
        .await
        .unwrap();

    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "test_anthropic_model_extracted_from_body: expected one cache entry"
    );
    assert_eq!(
        entries[0].model.as_deref(),
        Some("claude-sonnet-4-20250514"),
        "Anthropic model must come from request body"
    );
}

#[tokio::test]
async fn test_openai_model_extracted_from_body() {
    let mock = MockUpstream::builder()
        .sse(
            200,
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n"],
        )
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test-key")
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();

    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "test_openai_model_extracted_from_body: expected one cache entry"
    );
    assert_eq!(
        entries[0].model.as_deref(),
        Some("gpt-4o"),
        "OpenAI model must come from request body"
    );
}
