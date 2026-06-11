# Step 02: Recording Sessions

## Context

This step runs the recording binary from step-01 against live provider APIs to capture
real SSE responses. The output is a set of committed TOML cassette files in
`tests/fixtures/`. All cassettes use synthetic test keys — no real API keys are stored.

## Prerequisites

- Step 01 complete: `lcp-record` binary and `MockResponse::Recorded` exist.
- Environment: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GEMINI_API_KEY`
  set (from `~/.pi/agent/auth.json` or `.env`).
- Keys are in `/tmp/lcp-test-keys.sh` (export-ready).

## Recording Command

```bash
source /tmp/lcp-test-keys.sh
export ANTHROPIC_API_KEY OPENAI_API_KEY OPENROUTER_API_KEY GEMINI_API_KEY

# Record all scenarios defined in tests/fixtures/scenarios.toml:
cargo test --test lcp-tests --features record -- record --all --out-dir tests/fixtures/

# Record a single scenario:
cargo test --test lcp-tests --features record -- record \
  --scenario anthropic_tool_use_input_json
```

## Scenarios to Capture

Run ALL 34 scenarios below. They are grouped by provider and ordered to minimize
rate-limit risk (pause 2s between requests to the same provider).

---

### Anthropic (8 scenarios)

| ID | Model | Scenario | Why capture |
|---|---|---|---|
| `ant_tool_use_input_json` | claude-haiku-4-5 | Tool call — fake in `input_json_delta` | Core restore path; the flush_safe_prefix split bug |
| `ant_thinking_and_text` | claude-sonnet-4-5 | Extended thinking + text | Multi-block with thinking_delta, text_delta, signature_delta ordering |
| `ant_multi_block_stop` | claude-haiku-4-5 | Two concurrent tool calls (multi-block) | VC-SSE-16 block-scope isolation in real traffic |
| `ant_text_only` | claude-haiku-4-5 | Plain text reply, no tool | text_delta path, message_stop terminal |
| `ant_message_delta_stop` | claude-haiku-4-5 | message_delta arrives before message_stop | Verify message_delta → Stream flush fires |
| `ant_empty_thinking` | claude-sonnet-4-5 | Thinking enabled, model produces no thought | Empty thinking block (content_block_start then content_block_stop with no delta) |
| `ant_error_rate_limit` | claude-haiku-4-5 | Trigger 429 (use invalid model to force error quickly) | Proxy error pass-through; cache must NOT store error responses |
| `ant_error_overloaded` | claude-haiku-4-5 | 529 Anthropic overloaded | Proxy 5xx pass-through |

**Request body template for tool-use scenarios (anthropic):**
```json
{
  "model": "{{MODEL}}",
  "max_tokens": 300,
  "stream": true,
  "tools": [{
    "name": "store_key",
    "description": "Store an API key value for later retrieval.",
    "input_schema": {
      "type": "object",
      "properties": {"key": {"type": "string"}},
      "required": ["key"]
    }
  }],
  "messages": [{
    "role": "user",
    "content": "Call store_key with this key: {{SECRET_PLACEHOLDER}}"
  }]
}
```

The `{{SECRET_PLACEHOLDER}}` is replaced with the doppel fake of the synthetic ANT key
(`sk-ant-api03-YLY9P1-...`) before sending to the upstream.

---

### OpenAI Chat Completions (7 scenarios)

| ID | Model | Scenario | Why capture |
|---|---|---|---|
| `oai_chat_tool_calls` | gpt-4.1-mini | Tool call — fake in `tool_calls.function.arguments` | Core restore path for Chat Completions |
| `oai_chat_colocated_finish` | gpt-4.1-mini | Final chunk has `content:""` + `finish_reason` | The actual OpenAI wire format for final chunks |
| `oai_chat_content_only` | gpt-4.1-mini | Plain text, no tool | content delta → finish_reason → [DONE] path |
| `oai_chat_o4mini_reasoning` | o4-mini | o4-mini reasoning (reasoning_content field) | `reasoning_content` field — captured previously; verify not broken |
| `oai_chat_multi_tool` | gpt-4.1-mini | Two parallel tool calls | Multiple `tool_calls` indices in same stream |
| `oai_chat_stream_error` | gpt-4.1-mini | 400 Bad Request (malformed body) | Proxy 4xx pass-through without caching |
| `oai_chat_finish_stop` | gpt-4.1-mini | `finish_reason: stop` (no tool) | Verify `[DONE]` flush fires after finish_reason |

**Request body template:**
```json
{
  "model": "{{MODEL}}",
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
  "messages": [{"role": "user", "content": "Call store_key with key: {{SECRET_PLACEHOLDER}}"}]
}
```

---

### OpenAI Responses API (5 scenarios)

| ID | Model | Scenario | Why capture |
|---|---|---|---|
| `oai_resp_text_delta` | gpt-4.1-mini | Text output — fake in `output_text.delta` chunks | Core Responses API restore path |
| `oai_resp_done_sequence` | gpt-4.1-mini | Full sequence: delta → done → content_part.done → output_item.done → completed | Real event ordering; tests vc_sse_18 sub-cases |
| `oai_resp_error_incomplete` | gpt-4.1-mini | `response.incomplete` (max_output_tokens hit) | VC-SSE-18b: error terminal flush |
| `oai_resp_output_text_done_body` | gpt-4.1-mini | `response.output_text.done` with text body | Known gap: does the text in done body contain the fake? Probe and document |
| `oai_resp_completed_body` | gpt-4.1-mini | `response.completed` with full response body | Known gap: response.completed body leaks fake — capture exact format |

**Request body template:**
```json
{
  "model": "{{MODEL}}",
  "max_output_tokens": 200,
  "stream": true,
  "input": "Repeat exactly: {{SECRET_PLACEHOLDER}}"
}
```

---

### OpenRouter (8 scenarios)

| ID | Model | Scenario | Why capture |
|---|---|---|---|
| `or_claude_tool_use` | anthropic/claude-haiku-4-5 | Tool call — fake in arguments | Claude via OpenRouter; confirms `content:""` fix works |
| `or_claude_finish_colocated` | anthropic/claude-haiku-4-5 | Co-located `content:""` + `finish_reason` | **The exact bug scenario from Jun 2026 E2E** |
| `or_claude_text` | anthropic/claude-3-5-haiku | Plain text streaming | Verify OpenRouter normalizes Anthropic to Chat Completions format |
| `or_deepseek_chat` | deepseek/deepseek-chat-v3-0324 | Tool call | deepseek-chat via OpenRouter; verify SiliconFlow provider formatting |
| `or_deepseek_r1_reasoning` | deepseek/deepseek-r1-0528 | Reasoning — `reasoning` field (not `reasoning_content`) | **NEW field name**: DeepSeek R1 uses `delta.reasoning`, not `delta.reasoning_content` |
| `or_processing_prefix` | anthropic/claude-haiku-4-5 | First chunk starts with `: OPENROUTER PROCESSING` | Verify `is_sse_first_chunk` handles comment-line prefix |
| `or_o4mini_tool` | openai/o4-mini | Tool call via OpenRouter | Same shape as direct OpenAI but provider-specific headers |
| `or_error_no_credits` | (any) | 402 Payment Required | OpenRouter-specific error; proxy pass-through |

---

### Gemini (6 scenarios)

| ID | Model | Scenario | Why capture |
|---|---|---|---|
| `gem_tool_call` | gemini-2.5-flash | Tool call — fake in `functionCall.args` | Core Gemini restore path |
| `gem_multi_part_thinking` | gemini-2.5-pro | Thinking + tool call | Multi-part response: thought parts + tool call parts |
| `gem_colocated_finish` | gemini-2.5-flash | Text frame with co-located `finishReason` | **Known gap probe**: confirms finishReason is dropped when restore active |
| `gem_text_only` | gemini-2.5-flash | Plain text streaming | Simple text accumulation + finishReason terminal |
| `gem_usage_metadata` | gemini-2.5-flash | Response includes `usageMetadata` | Verify usageMetadata passthrough in non-restore path |
| `gem_error_quota` | gemini-2.5-flash | 429 quota exceeded | Provider error pass-through without cache write |

---

## Recording Procedure (Detailed)

For each scenario, the `lcp-record` binary does the following. Implement it in
`tests/bin/record.rs` (behind `#[cfg(feature = "record")]`):

```
1. Load scenario from scenarios.toml by ID
2. Compute doppel fake:
     let pat = patterns::<secret_kind>();
     let sr = doppel_swap(synthetic_secret, &[pat]).unwrap();
     let fake_str = String::from_utf8(sr.entries[0].fake.clone()).unwrap();
3. Substitute {{SECRET_PLACEHOLDER}} → fake_str in body_template
4. Send HTTP request to upstream directly (no proxy):
     client.post(upstream + path)
       .header("content-type", "application/json")
       .header("<auth_header>", api_key)  // NOT stored
       .body(body_with_fake)
5. Stream response, collecting chunks:
     while let Some(chunk) = response.chunk().await? {
         chunks.push(chunk);
     }
6. Write cassette TOML:
     - body_chunks = chunks as escaped strings
     - strip request-id/x-request-id/cf-ray from response headers
     - store status code + content-type
7. Print summary: scenario, status, N chunks, total bytes
```

**Important recording rules:**

- The cassette stores the body **with the fake key** already substituted. Reason: the
  cassette player serves as the "upstream" during replay, which means the proxy has
  already swapped the original secret for the fake before the cassette player is consulted.
- NEVER store `Authorization`, `x-api-key`, `x-goog-api-key`, or any token header.
- Strip `request-id`, `x-request-id`, `x-cloud-trace-context`, `cf-ray`, `cf-cache-status`
  from response headers — these are per-request identifiers that would make the cassette
  non-deterministic.
- Keep `content-type`, `transfer-encoding`, `cache-control`.
- Body chunks: split on actual TCP chunk boundaries if possible, else split on SSE frame
  boundaries (`\n\n`). The latter is always safe since `MockUpstream::Recorded` streams
  chunks as-is.
- Verify the cassette round-trips: reload it and confirm `full_body()` is non-empty and
  contains the expected SSE events.

## Post-Recording Verification

After recording all 34 cassettes, run:

```bash
# Verify no real API keys leaked into fixture files
grep -r "sk-ant-api03-YLY9P1\|sk-v0zsmdzWw\|AIzavURt9l4\|sk-or-v1" tests/fixtures/
# Must find nothing (or only the synthetic test constants)

# Verify schema = 1 in all files
grep -rL "^schema = 1" tests/fixtures/**/*.toml
# Must find nothing

# Count scenarios
grep -r "^scenario = " tests/fixtures/**/*.toml | wc -l
# Must equal 34
```

## Acceptance Criteria

- [ ] All 34 cassette files exist under `tests/fixtures/`
- [ ] No real API keys appear in any cassette file (verified by grep)
- [ ] All cassettes have `schema = 1`
- [ ] All cassettes contain at least 3 body_chunks
- [ ] `cargo nextest run --test lcp-tests` exits 0 (existing tests unaffected)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] Recording session log written to `artifacts/recording-session-YYYY-MM-DD.md`
  listing each scenario, status code, chunk count, and any anomalies observed

## Anomaly Log (fill in during execution)

During the recording session, note any deviations from expected wire format:

- Provider sends different field names than SPEC describes
- Unexpected extra fields or event types
- Rate limits hit (which scenarios required retry)
- Cassettes that needed manual editing to remove per-request identifiers
- Any scenario that failed to capture (upstream error, timeout)

These anomalies are the seeds for new spec tests and known-gap documentation.
