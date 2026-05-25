use crate::common::{MockUpstream, TestHarness};

fn anthropic_sse_chunks() -> Vec<&'static str> {
    vec![
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ]
}

fn openai_sse_chunks() -> Vec<&'static str> {
    vec!["data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n"]
}

fn gemini_sse_chunks() -> Vec<&'static str> {
    vec![
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n",
    ]
}

#[tokio::test]
async fn test_anthropic_metadata_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, anthropic_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}],"metadata":{"user_id":"test-user-123"}}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let cache_status = resp2
        .headers()
        .get("x-lcp-cache")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_eq!(
        key1, key2,
        "Anthropic `metadata` must be stripped; keys must match"
    );
    assert_eq!(
        cache_status, "HIT",
        "second request must be a cache hit when metadata is stripped"
    );
}

#[tokio::test]
async fn test_anthropic_thinking_not_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, anthropic_sse_chunks())
        .sse(200, anthropic_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}],"thinking":{"type":"enabled","budget_tokens":1000}}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();

    let resp2 = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test")
        .header("content-type", "application/json")
        .body(r#"{"model":"claude-sonnet-4-20250514","max_tokens":10,"messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_ne!(
        key1, key2,
        "Anthropic `thinking` must NOT be stripped; keys must differ"
    );
}

#[tokio::test]
async fn test_openai_user_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, openai_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/openai/v1/chat/completions", harness.proxy_url()))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"user":"user-abc123"}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let resp2 = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let cache_status = resp2
        .headers()
        .get("x-lcp-cache")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_eq!(
        key1, key2,
        "OpenAI `user` must be stripped; keys must match"
    );
    assert_eq!(
        cache_status, "HIT",
        "second request must be a cache hit when user is stripped"
    );
}

#[tokio::test]
async fn test_openrouter_user_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, openai_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/openrouter/v1/chat/completions", harness.proxy_url()))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}],"user":"user-abc123"}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let resp2 = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(
            r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let cache_status = resp2
        .headers()
        .get("x-lcp-cache")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_eq!(
        key1, key2,
        "OpenRouter `user` must be stripped; keys must match"
    );
    assert_eq!(
        cache_status, "HIT",
        "second request must be a cache hit when user is stripped"
    );
}

#[tokio::test]
async fn test_openrouter_provider_route_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, openai_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/openrouter/v1/chat/completions", harness.proxy_url()))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}],"provider":{"order":["Anthropic"]},"route":"fallback"}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let resp2 = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(
            r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let cache_status = resp2
        .headers()
        .get("x-lcp-cache")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_eq!(
        key1, key2,
        "OpenRouter `provider` and `route` must be stripped; keys must match"
    );
    assert_eq!(
        cache_status, "HIT",
        "second request must be a cache hit when routing fields are stripped"
    );
}

#[tokio::test]
async fn test_openrouter_transforms_not_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, openai_sse_chunks())
        .sse(200, openai_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/openrouter/v1/chat/completions", harness.proxy_url()))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}],"transforms":["middle-out"]}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();

    let resp2 = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(
            r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_ne!(
        key1, key2,
        "OpenRouter `transforms` must NOT be stripped; keys must differ"
    );
}

#[tokio::test]
async fn test_openrouter_reasoning_not_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, openai_sse_chunks())
        .sse(200, openai_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!("{}/openrouter/v1/chat/completions", harness.proxy_url()))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}],"reasoning":{"effort":"high"}}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();

    let resp2 = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(
            r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_ne!(
        key1, key2,
        "OpenRouter `reasoning` must NOT be stripped; keys must differ"
    );
}

#[tokio::test]
async fn test_cross_provider_same_body_different_keys() {
    let mock = MockUpstream::builder()
        .sse(200, openai_sse_chunks())
        .sse(200, openai_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;

    let resp1 = client
        .post(format!(
            "{}/openai/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();

    let resp2 = client
        .post(format!(
            "{}/openrouter/v1/chat/completions",
            harness.proxy_url()
        ))
        .header("authorization", "Bearer test")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_ne!(
        key1, key2,
        "same body routed through different providers must produce different cache keys"
    );
}

#[tokio::test]
async fn test_gemini_no_extra_fields_stripped() {
    let mock = MockUpstream::builder()
        .sse(200, gemini_sse_chunks())
        .sse(200, gemini_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    let resp1 = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-2.5-flash:generateContent",
            harness.proxy_url()
        ))
        .query(&[("key", "test-api-key")])
        .header("content-type", "application/json")
        .body(r#"{"contents":[{"parts":[{"text":"hi"}]}],"generationConfig":{"topK":40}}"#)
        .send()
        .await
        .unwrap();
    let key1 = resp1
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp1.bytes().await.unwrap();

    let resp2 = client
        .post(format!(
            "{}/gemini/v1beta/models/gemini-2.5-flash:generateContent",
            harness.proxy_url()
        ))
        .query(&[("key", "test-api-key")])
        .header("content-type", "application/json")
        .body(r#"{"contents":[{"parts":[{"text":"hi"}]}]}"#)
        .send()
        .await
        .unwrap();
    let key2 = resp2
        .headers()
        .get("x-lcp-key")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    resp2.bytes().await.unwrap();

    assert_ne!(
        key1, key2,
        "Gemini has no extra strip fields; generationConfig must not be stripped, keys must differ"
    );
}
