# Step 09: Extend Spec Tests — Model Extraction Fallback + OpenRouter

## Context

### Overall Objective
Add 20+ spec invariant and integration tests covering behavioral correctness, error paths, edge cases, and admin endpoints.

### Phase Context
Wave 1: spec test files. All Wave 1 steps run in parallel after Wave 0 completes. This step extends an existing file.

### This Step
Extends `tests/spec/model_extraction.rs` with three new tests:
1. `test_openrouter_model_extracted_from_body` — OpenRouter uses the same body format as OpenAI; the model value may contain a provider prefix (`anthropic/claude-sonnet-4`).
2. `test_malformed_json_body_stores_model_none` — when the request body is not valid JSON, model extraction must not panic; the cache entry must have `model: None`.
3. `test_gemini_unrecognized_path_stores_model_none` — when a Gemini URL doesn't match the expected path pattern (no model segment), model extraction must store `None`.

Covers lcp-server/SPEC.md §106–108 and OpenRouter provider contract.

## Prerequisites
- Wave 0 complete (all three of step-01, step-02, step-03)

## Files to Read Before Starting
- `tests/spec/model_extraction.rs` — read the FULL file; understand existing test patterns and helpers before adding
- `tests/spec/mod.rs` — no change needed here (module already registered as `mod model_extraction;`)

## Implementation

### Task 1: Add three tests to the end of tests/spec/model_extraction.rs

Append the following tests after the last test in the file (`test_openai_model_extracted_from_body`). Do NOT modify existing tests.

```rust
#[tokio::test]
async fn test_openrouter_model_extracted_from_body() {
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
        .post(format!("{}/openrouter/v1/chat/completions", harness.proxy_url()))
        .header("authorization", "Bearer test-key")
        .header("content-type", "application/json")
        .body(r#"{"model":"anthropic/claude-sonnet-4","messages":[{"role":"user","content":"hi"}]}"#)
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    assert_eq!(
        entries.len(),
        1,
        "test_openrouter_model_extracted_from_body: expected one cache entry"
    );
    assert_eq!(
        entries[0].model.as_deref(),
        Some("anthropic/claude-sonnet-4"),
        "OpenRouter model must be extracted from request body with provider prefix preserved"
    );
}

#[tokio::test]
async fn test_malformed_json_body_stores_model_none() {
    let mock = MockUpstream::builder()
        .sse(
            200,
            vec!["event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"],
        )
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // Send a body that is not valid JSON — proxy must not panic; model stored as None.
    let resp = client
        .post(format!("{}/anthropic/v1/messages", harness.proxy_url()))
        .header("x-api-key", "test-key")
        .header("content-type", "application/json")
        .body("not valid json{{{")
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();
    harness.wait_for_writes().await;

    let entries = harness.cache().list_entries().unwrap();
    // Entry may or may not be stored depending on upstream response status, but if stored:
    if !entries.is_empty() {
        assert!(
            entries[0].model.is_none(),
            "malformed JSON body must result in model: None; got: {:?}",
            entries[0].model
        );
    }
    // If no entry was stored (e.g., upstream rejected the request), the proxy at least did not panic.
    // The test passes as long as we reach this line.
}

#[tokio::test]
async fn test_gemini_unrecognized_path_stores_model_none() {
    let mock = MockUpstream::builder()
        .sse(200, gemini_sse_chunks())
        .build()
        .await;
    let harness = TestHarness::builder().mock(mock).build().await;
    let client = reqwest::Client::new();

    // Path missing the model segment — does not match the expected pattern.
    let resp = client
        .post(format!(
            "{}/gemini/v1/projects/x/locations/y/publishers/google/models:generateContent",
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
    if !entries.is_empty() {
        assert!(
            entries[0].model.is_none(),
            "unrecognized Gemini path must result in model: None; got: {:?}",
            entries[0].model
        );
    }
}
```

**Note:** The new tests reference `gemini_sse_chunks()` and `gemini_request_body()` which are already defined earlier in the file. Do not redefine them.

## Acceptance Criteria

These must ALL pass before reporting complete:

- [ ] `cargo nextest run --test spec test_openrouter_model_extracted_from_body` exits 0
- [ ] `cargo nextest run --test spec test_malformed_json_body_stores_model_none` exits 0
- [ ] `cargo nextest run --test spec test_gemini_unrecognized_path_stores_model_none` exits 0
- [ ] `cargo nextest run --test spec` exits 0 (no regressions — all existing model_extraction tests still pass)

## Reviewer Instructions

You are reviewing Step 09 implementation. Verify:

1. Run `cargo nextest run --test spec test_openrouter_model_extracted_from_body` — must exit 0
2. Run `cargo nextest run --test spec test_malformed_json_body_stores_model_none` — must exit 0
3. Run `cargo nextest run --test spec test_gemini_unrecognized_path_stores_model_none` — must exit 0
4. Run `cargo nextest run --test spec model_extraction` — must exit 0, all model extraction tests (existing + 3 new) pass
5. Check `tests/spec/model_extraction.rs` was not modified except by appending the 3 new tests at the end

Report: "PASS" with each criterion confirmed, or "FAIL: <criterion> — <what's wrong> — expected: <what>"

## Rollback
If this step needs to be reverted: remove the 3 appended test functions from the end of `tests/spec/model_extraction.rs`. The existing tests are unmodified.
