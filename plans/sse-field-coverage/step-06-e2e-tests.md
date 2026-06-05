# Step 06: E2E Tests — Real Provider Validation

## Context

After Waves 1–4, all new SSE fields are handled with integration tests using synthetic SSE streams. This step adds E2E tests against real provider APIs to validate end-to-end behavior through the proxy.

Each test is gated by an environment variable for the provider's API key. If the env var is absent, the test returns early with a skip message (not a failure).

## Prerequisites

- **step-02** (Anthropic), **step-03** (OpenAI/OpenRouter), **step-04** (Responses API), **step-05** (Gemini) must all be complete.

## Files to Read Before Starting

- `tests/e2e/mod.rs` — entry point for E2E tests; see existing pattern.
- `tests/e2e/cli.rs` — existing E2E test for structure/pattern reference.
- `tests/Cargo.toml` — the `test-e2e` feature gate.
- `/home/ignacio/pr/llm-cache-proxy/lcp-provider-nuances.md` — model names, endpoints, auth, token params, and pitfalls.
- `tests/common/mod.rs` — `TestHarness` and `MockUpstream` (for integration tests; E2E tests use the real proxy + real upstream instead).

## Implementation

### 1. Create E2E test module files

Create `tests/e2e/sse_fields.rs` and add `mod sse_fields;` to `tests/e2e/mod.rs`.

### 2. Implement the skip-if-no-key pattern

Each test starts with:
```rust
fn require_env(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("skipping: {var} not set");
            None
        }
    }
}
```

Use at the top of each test: `let Some(api_key) = require_env("ANTHROPIC_API_KEY") else { return; };`

### 3. E2E test infrastructure

Each test must:
1. Start a `TestHarness` with `DoppelExt` configured with the relevant patterns
2. Send a real API request through the proxy to the real provider
3. The request body contains a swapped secret (via `doppel::swap`)
4. The response streams through the proxy, which restores fakes → originals
5. Assert the response contains the original secret and not the fake

The `TestHarness` can be configured to use a real upstream (not `MockUpstream`). Check how `TestHarness::builder()` works without `.mock(...)` — if it requires a mock, you'll need to pass requests through to the real API. Look at the existing E2E tests for the pattern.

If `TestHarness` doesn't support real upstreams directly, the E2E test can start the proxy binary and send requests to it. Check `tests/e2e/cli.rs` for the pattern.

### 4. E2E tests

#### Anthropic

**`e2e_anthropic_thinking_and_text`:**
- **Env var:** `ANTHROPIC_API_KEY`
- **Model:** `claude-sonnet-4-6`
- **Endpoint:** `/anthropic/v1/messages`
- **Auth:** `x-api-key: <key>`, `anthropic-version: 2023-06-01`
- **Request:** `max_tokens: 2000`, `thinking: { type: "enabled", budget_tokens: 2000 }`, `stream: true`, message containing the secret
- **Assert:** Response contains original secret. Response does not contain the fake. Response must contain `thinking_delta` events (verify by checking for `"thinking_delta"` in the response body).
- **Fields exercised:** `delta.thinking`, `delta.text`
- **Pattern:** `patterns::anthropic()`, secret = `ANT`

**`e2e_anthropic_tool_use`:**
- **Env var:** `ANTHROPIC_API_KEY`
- **Model:** `claude-haiku-4-5`
- **Endpoint:** `/anthropic/v1/messages`
- **Auth:** Same as above
- **Request:** Define a tool with a string parameter (e.g., `{"name": "store_key", "description": "Store an API key", "input_schema": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}}`). Prompt: "Call store_key with this key: {secret}". `max_tokens: 500`, `stream: true`.
- **Assert:** Response contains original secret in `input_json_delta` events.
- **Fields exercised:** `delta.partial_json`
- **Pattern:** `patterns::anthropic()`, secret = `ANT`

#### OpenAI

**`e2e_openai_tool_calls`:**
- **Env var:** `OPENAI_API_KEY`
- **Model:** `gpt-4o-mini`
- **Endpoint:** `/openai/v1/chat/completions`
- **Auth:** `Authorization: Bearer <key>`
- **Request:** Define a function tool: `{"type": "function", "function": {"name": "store_key", "parameters": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}}}`. Prompt: "Call store_key with key: {secret}". `max_tokens: 200`, `stream: true`.
- **Assert:** Response contains original secret in `tool_calls[0].function.arguments`.
- **Fields exercised:** `tool_calls[N].function.arguments`
- **Pattern:** `patterns::openai_classic()`, secret = `OPENAI_CLASSIC`

**`e2e_openai_reasoning_content`:**
- **Env var:** `OPENAI_API_KEY`
- **Model:** `o4-mini`
- **Endpoint:** `/openai/v1/chat/completions`
- **Auth:** `Authorization: Bearer <key>`
- **Request:** `max_completion_tokens: 2000`, `reasoning_effort: "low"`, `stream: true`. Prompt containing the secret.
- **Assert:** Response contains original secret. Response contains `reasoning_content` fields (verify by checking response body for `"reasoning_content"`).
- **Fields exercised:** `reasoning_content`
- **Pattern:** `patterns::openai_classic()`, secret = `OPENAI_CLASSIC`

**`e2e_openai_responses_api`:**
- **Env var:** `OPENAI_API_KEY`
- **Model:** `gpt-5.5-pro` (Responses API tier; see `lcp-provider-nuances.md`)
- **Endpoint:** `/openai/v1/responses`
- **Auth:** `Authorization: Bearer <key>`
- **Request:** `{"model": "gpt-5.5-pro", "input": "Repeat exactly: {secret}", "max_output_tokens": 2000, "stream": true}`
- **Assert:** Response contains original secret. Response contains `response.output_text.delta` events.
- **Fields exercised:** Responses API `delta`, `text`
- **Pattern:** `patterns::openai_classic()`, secret = `OPENAI_CLASSIC`

#### OpenRouter

**`e2e_openrouter_reasoning`:**
- **Env var:** `OPENROUTER_API_KEY`
- **Model:** `openai/o4-mini`
- **Endpoint:** `/openrouter/v1/chat/completions`
- **Auth:** `Authorization: Bearer <key>`
- **Request:** `max_completion_tokens: 2000`, `stream: true`. Prompt containing the secret.
- **Assert:** Response contains original secret. Response contains `reasoning_content` fields.
- **Fields exercised:** `reasoning_content`
- **Pattern:** `patterns::openai_classic()`, secret = `OPENAI_CLASSIC`

**`e2e_openrouter_tool_calls`:**
- **Env var:** `OPENROUTER_API_KEY`
- **Model:** `openai/gpt-4o-mini`
- **Endpoint:** `/openrouter/v1/chat/completions`
- **Auth:** `Authorization: Bearer <key>`
- **Request:** Same tool definition as OpenAI test. `max_tokens: 200`, `stream: true`.
- **Assert:** Response contains original secret in tool_calls arguments.
- **Fields exercised:** `tool_calls[N].function.arguments`
- **Pattern:** `patterns::openai_classic()`, secret = `OPENAI_CLASSIC`

#### Gemini

**`e2e_gemini_multi_part_thinking`:**
- **Env var:** `GEMINI_API_KEY`
- **Model:** `gemini-2.5-pro`
- **Endpoint:** `/gemini/v1beta/models/gemini-2.5-pro:streamGenerateContent`
- **Auth:** API key as `?key=` query param
- **Request:** `{"contents": [{"parts": [{"text": "Repeat: {secret}"}]}], "generationConfig": {"maxOutputTokens": 2000, "thinkingConfig": {"includeThoughts": true}}, "systemInstruction": {"parts": [{"text": "Think step by step before answering."}]}}`
- **Assert:** Response contains original secret. Response contains multiple parts (thought + answer).
- **Fields exercised:** `parts[N].text` for N > 0, including thought parts
- **Pattern:** `patterns::gcp()`, secret = `GCP`

**`e2e_gemini_tool_call`:**
- **Env var:** `GEMINI_API_KEY`
- **Model:** `gemini-2.5-flash`
- **Endpoint:** `/gemini/v1beta/models/gemini-2.5-flash:streamGenerateContent`
- **Auth:** API key as `?key=` query param
- **Request:** Define a function declaration: `{"tools": [{"functionDeclarations": [{"name": "store_key", "description": "Store a key", "parameters": {"type": "OBJECT", "properties": {"key": {"type": "STRING"}}, "required": ["key"]}}]}]}`. Prompt: "Call store_key with key: {secret}". `maxOutputTokens: 1000`.
- **Assert:** Response contains original secret in `functionCall.args` values.
- **Fields exercised:** `functionCall.args.*`
- **Pattern:** `patterns::gcp()`, secret = `GCP`

## Acceptance Criteria

1. **E2E tests compile without errors:**
   ```
   cargo nextest run -p lcp-tests --test e2e --features test-e2e --no-run
   ```

2. **E2E tests pass when API keys are available (manual verification):**
   ```
   ANTHROPIC_API_KEY=... OPENAI_API_KEY=... OPENROUTER_API_KEY=... GEMINI_API_KEY=... \
     cargo nextest run -p lcp-tests --test e2e --features test-e2e -E 'test(e2e_)'
   ```

3. **E2E tests skip gracefully when API keys are absent:**
   ```
   cargo nextest run -p lcp-tests --test e2e --features test-e2e -E 'test(e2e_)'
   ```
   All tests must pass (not fail) — they return early with a skip message.

4. **No clippy warnings:**
   ```
   cargo clippy -p lcp-tests --all-targets --features test-e2e -- -D warnings
   ```

## Reviewer Instructions

1. Run acceptance criteria #1 (compile check) and #3 (skip check).
2. If API keys are available, run #2 and verify at least one provider's tests pass.
3. Verify each test function:
   - Uses the correct model from `lcp-provider-nuances.md`
   - Uses the correct endpoint path
   - Uses the correct auth mechanism
   - Uses appropriate token budget (2000+ for reasoning/thinking models)
   - Asserts both `assert_present(original)` and `assert_absent(fake)`
4. Verify the skip-if-no-key pattern returns early (not panic/fail).

## Rollback

```
git checkout HEAD -- tests/e2e/
```
