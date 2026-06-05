# Step 07: Cross-Field Isolation (VC-SSE-13) + Full Regression Sweep

## Context

This step adds integration tests for VC-SSE-13 (cross-field isolation) and runs the complete regression sweep. Cross-field isolation verifies that when a response contains multiple independent content fields, fakes in each field are restored independently and a partial fake at the end of one field's buffer does not match against another field's buffer.

## Prerequisites

- **step-02** (Anthropic), **step-03** (OpenAI/OpenRouter), **step-04** (Responses API), **step-05** (Gemini) must all be complete.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — verify the multi-buffer accumulation uses separate `FieldKey`s per content field.
- `crates/lcp-server/SPEC.md` §Multi-Field Accumulation and VC-SSE-13.
- `tests/integration/doppel.rs` — existing test patterns and helpers.

## Implementation

### 1. Integration test: `cross_field_isolation_openai_content_and_tool_calls` (VC-SSE-13)

This test uses **two different secrets** — one swapped into `content` fields, another into `tool_calls.arguments` fields. Both must be independently restored.

**Setup:**
```rust
use doppel::swap as doppel_swap;

// Secret 1 for content field
let pat1 = patterns::openai_classic();
let secret1 = OPENAI_CLASSIC;
let body1 = [b"key: ".as_slice(), secret1].concat();
let sr1 = doppel_swap(&body1, std::slice::from_ref(&pat1)).unwrap();
let fake1 = sr1.entries[0].fake.clone();
let fake1_str = String::from_utf8_lossy(&fake1).into_owned();

// Secret 2 for tool_calls field (use a different secret)
let pat2 = patterns::github_classic();
let secret2 = GITHUB_CLASSIC;
let body2 = [b"token: ".as_slice(), secret2].concat();
let sr2 = doppel_swap(&body2, std::slice::from_ref(&pat2)).unwrap();
let fake2 = sr2.entries[0].fake.clone();
let fake2_str = String::from_utf8_lossy(&fake2).into_owned();
```

Split each fake into 2 parts (simpler than 4 — cross-field isolation doesn't need fine granularity).

**SSE stream:**
```
data: {"id":"chatcmpl-x","choices":[{"index":0,"delta":{"content":"prefix-<CONTENT_PART1>"}}]}

data: {"id":"chatcmpl-x","choices":[{"index":0,"delta":{"content":"<CONTENT_PART2>-suffix"}}]}

data: {"id":"chatcmpl-x","choices":[{"index":0,"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"fn1","arguments":""}}]}}]}

data: {"id":"chatcmpl-x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"<TOOL_PART1>"}}]}}]}

data: {"id":"chatcmpl-x","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"<TOOL_PART2>"}}]}}]}

data: [DONE]
```

**Assertions:**
1. `assert_present(&resp_bytes, &[secret1], "content field: original must be restored")`
2. `assert_present(&resp_bytes, &[secret2], "tool_calls field: original must be restored")`
3. `assert_absent(&resp_bytes, &[&fake1, &fake2], "no fakes in output")`

Post to `/openai/v1/chat/completions`. Register both patterns in `DoppelExt::new(vec![pat1, pat2])`.

### 2. Integration test: `cross_field_isolation_anthropic_text_and_thinking` (VC-SSE-13)

Uses **two different secrets** — one in `thinking_delta` events, another in `text_delta` events.

**Setup:**
```rust
// Secret 1 for thinking
let pat1 = patterns::anthropic();
let secret1 = ANT;
// ... swap → fake1

// Secret 2 for text content
let pat2 = patterns::github_classic();
let secret2 = GITHUB_CLASSIC;
// ... swap → fake2
```

Split each fake into 2 parts.

**SSE stream:**
```
event: message_start
data: {"type":"message_start","message":{"id":"msg_test","type":"message","role":"assistant","content":[],"stop_reason":null}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"<THINK_PART1>"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"<THINK_PART2>"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"<TEXT_PART1>"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"<TEXT_PART2>"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}

event: message_stop
data: {"type":"message_stop"}
```

**Assertions:**
1. `assert_present(&resp_bytes, &[secret1], "thinking field: original must be restored")`
2. `assert_present(&resp_bytes, &[secret2], "text field: original must be restored")`
3. `assert_absent(&resp_bytes, &[&fake1, &fake2], "no fakes in output")`

Post to `/anthropic/v1/messages`. Register both patterns.

### 3. Full regression sweep

After the cross-field tests are written and passing, run the complete test suite:

```bash
cargo nextest run --workspace
```

Every test must pass. If any test fails, investigate and fix before marking this step complete.

## Acceptance Criteria

1. **Cross-field isolation tests pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(cross_field_isolation_openai_content_and_tool_calls)'
   cargo nextest run -p lcp-tests --test integration -E 'test(cross_field_isolation_anthropic_text_and_thinking)'
   ```

2. **All existing SSE integration tests still pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_anthropic_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_anthropic_sse_stream_with_event_prefix)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openai_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_gemini_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openrouter_sse_stream)'
   ```

3. **All new integration tests from steps 02-05 still pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_anthropic_thinking_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_anthropic_input_json_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(passthrough_anthropic_signature_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_tool_calls_arguments)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_reasoning_content)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openrouter_reasoning_content)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_deprecated_function_call)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_responses_api_text_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_responses_api_reasoning_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_gemini_multi_part_text)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_gemini_code_execution_output)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_gemini_function_call_args)'
   cargo nextest run -p lcp-tests --test integration -E 'test(passthrough_gemini_metadata_fields)'
   ```

4. **Full workspace green:**
   ```
   cargo nextest run --workspace
   ```

5. **No clippy warnings:**
   ```
   cargo clippy --workspace --all-targets -- -D warnings
   ```

6. **VCs satisfied:**
   - VC-SSE-13: Both `cross_field_isolation_*` tests pass

## Reviewer Instructions

1. Run acceptance criteria #1, #4, and #5. All must exit 0.
2. Read both cross-field isolation tests:
   - Verify they use **two different secrets** (not the same secret in both fields)
   - Verify they use **two different patterns** (different key types)
   - Verify the SSE stream interleaves fields correctly
   - Verify assertions check both originals present and both fakes absent
3. Verify the full workspace test suite passes with zero failures.

## Rollback

```
git checkout HEAD -- tests/integration/doppel.rs
```
