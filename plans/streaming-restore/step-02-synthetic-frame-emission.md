# Step 2: Synthetic Frame Emission + Per-FieldKey Flush

## Objective

Implement `build_synthetic_sse_frame` to produce valid provider-specific SSE frames from restored text, and ensure the per-FieldKey flush produces output that downstream parsers accept.

## Prerequisite

Step 1 complete (state machine works, safe-prefix flush emits something).

## Scope

If Step 1 already produces correct synthetic frames and all VC-SSE-1..13 tests pass, this step may be a no-op or only refinement. The step exists to handle the case where Step 1 deferred detailed frame construction.

## Design

### `build_synthetic_sse_frame(key: &FieldKey, restored_text: &str, provider: Provider) -> Vec<u8>`

Constructs a minimal valid SSE frame with the restored text placed in the correct JSON path for the given FieldKey.

**Per FieldKey variant:**

| FieldKey | `event:` line | JSON template |
|---|---|---|
| `AnthropicDelta { delta_type: "text_delta", index: N }` | `event: content_block_delta` | `{"type":"content_block_delta","index":N,"delta":{"type":"text_delta","text":"<TEXT>"}}` |
| `AnthropicDelta { delta_type: "thinking_delta", index: N }` | `event: content_block_delta` | `{"type":"content_block_delta","index":N,"delta":{"type":"thinking_delta","thinking":"<TEXT>"}}` |
| `AnthropicDelta { delta_type: "input_json_delta", index: N }` | `event: content_block_delta` | `{"type":"content_block_delta","index":N,"delta":{"type":"input_json_delta","partial_json":"<TEXT>"}}` |
| `OpenAiContent` | _(none)_ | `{"choices":[{"delta":{"content":"<TEXT>"}}]}` |
| `OpenAiToolCall { index: N }` | _(none)_ | `{"choices":[{"delta":{"tool_calls":[{"index":N,"function":{"arguments":"<TEXT>"}}]}}]}` |
| `OpenAiReasoning` | _(none)_ | `{"choices":[{"delta":{"reasoning_content":"<TEXT>"}}]}` |
| `OpenAiFunctionCallArgs` | _(none)_ | `{"choices":[{"delta":{"function_call":{"arguments":"<TEXT>"}}}]}` |
| `OpenAiRefusal` | _(none)_ | `{"choices":[{"delta":{"refusal":"<TEXT>"}}]}` |
| `ResponsesApiDelta { event_type }` | `event: <event_type>` | `{"delta":"<TEXT>"}` |
| `ResponsesApiDone { event_type }` | `event: <event_type>` | `{"text":"<TEXT>"}` |
| `GeminiText { thought: bool }` | _(none)_ | `{"candidates":[{"content":{"parts":[{"text":"<TEXT>"}]}}]}` |

**Output format:** `event: <type>\ndata: <json>\n\n` (with `event:` line only when specified).

**JSON encoding:** The `<TEXT>` value MUST be properly JSON-escaped (handle `"`, `\`, newlines, control characters). Use `serde_json::to_string(text)` to produce the escaped string value (includes surrounding quotes), then strip the quotes and embed in the template. Or use `serde_json::Value::String(text.to_string())` and serialize the whole object.

**Preferred approach:** Build a `serde_json::Value` object programmatically and serialize with `serde_json::to_string`. This handles all escaping correctly.

### Integration with `flush_safe_prefix`

`flush_safe_prefix` (from Step 1) calls `build_synthetic_sse_frame` for each FieldKey that has a safe prefix to emit. The returned `Vec<u8>` is wrapped in `Bytes::from()` and added to the output queue.

### Ordering within one poll cycle

When multiple FieldKeys flush in the same cycle, the order of synthetic frames in `output_queue` is deterministic but arbitrary. The spec permits this: "Frame granularity in the client-visible stream MAY differ from the original."

For determinism in tests, iterate accumulators in a consistent order. If using `HashMap`, switch to `BTreeMap<FieldKey, String>` or sort keys before iterating. `BTreeMap` is preferred for natural ordering.

## Edge Cases

1. **Empty restored text after restore:** If `doppel::restore` produces empty output for a non-empty input, still emit a frame (the text was all fake content that got replaced with nothing — this shouldn't happen with correct entries, but handle gracefully).

2. **Multi-line restored text:** The restored text may contain `\n` characters. In SSE, a single `data:` field can span multiple lines using `data: line1\ndata: line2\n`. However, since we're emitting a JSON object in the `data:` field, newlines inside the JSON string are escaped as `\n` (two characters) by `serde_json`, so the `data:` line remains a single line. No special handling needed.

3. **`[DONE]` sentinel:** Pass-through frames (including `data: [DONE]`) are handled in `process_sse_frame` — they bypass the accumulator entirely and go straight to `output_queue`.

## Verification

```bash
# All VC-SSE-1..13 must pass (correct JSON structure in output)
cargo nextest run --test integration

# Spec invariant tests pass
cargo nextest run --test spec -E 'test(sse_restore_streaming)'

# Inline unit tests — especially sse_event_lines_preserved_through_text_frame_reconstruction
cargo nextest run -p lcp-server

# Clippy clean
cargo clippy --workspace --all-targets -- -D warnings
```

## Exit Criteria

- `build_synthetic_sse_frame` produces valid SSE frames for all FieldKey variants.
- All VC-SSE-1..13 integration tests pass with correct content.
- The inline test `sse_event_lines_preserved_through_text_frame_reconstruction` passes (or is adjusted if output chunking changed — content must be identical).
- `cargo clippy` clean.
