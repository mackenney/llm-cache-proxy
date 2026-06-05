# Step 01: Refactor SSE Restore Internals — Multi-Buffer Architecture

## Context

The SSE restore pipeline currently uses a single-field model: `extract_text_field` returns one `Option<&str>` per event, everything goes into one `text_buf`, one `restore_stream` call, and redistribution puts all text into the first text frame. This cannot handle multiple content fields per event (Gemini multi-part, OpenAI tool_calls + content) or different field schemas (Responses API).

This step replaces the internals with a keyed multi-buffer model. **No new provider fields are added** — this step must produce the exact same observable behavior as before. The only change is architectural: the code is restructured to support multiple fields per event, but only the existing single field per provider is actually extracted.

## Prerequisites

None — this is the first step.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — the entire file (640 lines). Understand `extract_text_field`, `set_text_field`, `ParsedFrame`, `restore_sse`, the frame loop, and the redistribution logic.
- `tests/integration/doppel.rs` lines 700–1067 — the 5 existing SSE integration tests. These are the regression suite.

## Implementation

### 1. Define new types (top of `sse_restore.rs`, after imports, before functions)

```rust
/// Identifies a logically independent content stream within an SSE response.
/// Fields with the same key accumulate into the same buffer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FieldKey {
    // Anthropic — keyed by (delta_type, content_block_index)
    AnthropicDelta { delta_type: String, index: u64 },
    // OpenAI / OpenRouter chat completions
    OpenAiContent,
    OpenAiToolCall { index: u64 },
    OpenAiReasoning,
    OpenAiFunctionCallArgs,
    // OpenAI Responses API
    ResponsesApiDelta { event_type: String },
    ResponsesApiDone { event_type: String },
    // Gemini
    GeminiText { thought: bool },
    GeminiCodeExecOutput,
    GeminiFuncCallArg { arg_key: String },
}
```

```rust
/// A content-bearing field extracted from one SSE event.
struct ExtractedField {
    key: FieldKey,
    text: String,
    /// Write-back location: the information needed to set the restored text
    /// back into the correct JSON path. This is separate from `key` because
    /// the accumulation identity may differ from the write-back path (e.g.,
    /// Gemini parts keyed by `thought` but written back by `part_index`).
    write_back: WriteBackInfo,
}
```

```rust
/// Information needed to write a restored value back into its JSON location.
#[derive(Debug, Clone)]
enum WriteBackInfo {
    /// Anthropic: `json["delta"][field_name]` where field_name is "text", "thinking", or "partial_json"
    AnthropicDelta { field_name: String },
    /// OpenAI: `json["choices"][0]["delta"]["content"]`
    OpenAiContent,
    /// OpenAI: `json["choices"][0]["delta"]["tool_calls"][array_pos]["function"]["arguments"]`
    /// `array_pos` is the position within the `tool_calls` array in *this* event's JSON.
    OpenAiToolCall { array_pos: usize },
    /// OpenAI: `json["choices"][0]["delta"]["reasoning_content"]`
    OpenAiReasoning,
    /// OpenAI: `json["choices"][0]["delta"]["function_call"]["arguments"]`
    OpenAiFunctionCallArgs,
    /// Responses API: `json["delta"]`
    ResponsesApiDelta,
    /// Responses API: `json["text"]`
    ResponsesApiDone,
    /// Gemini: `json["candidates"][0]["content"]["parts"][part_index]["text"]`
    GeminiPartText { part_index: usize },
    /// Gemini: `json["candidates"][0]["content"]["parts"][part_index]["codeExecutionResult"]["output"]`
    GeminiPartCodeExecOutput { part_index: usize },
    /// Gemini: `json["candidates"][0]["content"]["parts"][part_index]["functionCall"]["args"][arg_key]`
    GeminiPartFuncCallArg { part_index: usize, arg_key: String },
}
```

```rust
/// Records what a single frame contributed to each accumulation buffer.
struct FrameFieldContribution {
    key: FieldKey,
    byte_len: usize,
    write_back: WriteBackInfo,
}
```

### 2. Implement `extract_fields`

```rust
fn extract_fields(json: &Value, provider: Provider, _event_type: Option<&str>) -> Vec<ExtractedField>
```

For this step, implement **only the existing extraction paths** (the ones `extract_text_field` currently handles):

- **Anthropic:** Check `type == "content_block_delta"` and `delta.type == "text_delta"`. Extract `delta.text`. Key = `AnthropicDelta { delta_type: "text_delta".into(), index }` where `index` comes from the top-level `index` field (default 0). WriteBack = `AnthropicDelta { field_name: "text".into() }`.
- **OpenAI / OpenRouter:** Extract `choices[0].delta.content`. Key = `OpenAiContent`. WriteBack = `OpenAiContent`.
- **Gemini:** Extract `candidates[0].content.parts[0].text`. Key = `GeminiText { thought: false }`. WriteBack = `GeminiPartText { part_index: 0 }`.

Return `Vec<ExtractedField>` with 0 or 1 elements (matching the old `Option<&str>` cardinality).

### 3. Implement `apply_restored_fields`

```rust
fn apply_restored_fields(json: &mut Value, restorations: &[(WriteBackInfo, String)]) -> Result<(), String>
```

For each `(write_back, text)`, navigate to the JSON path described by `write_back` and set the string value. For this step, only `AnthropicDelta { field_name: "text" }`, `OpenAiContent`, and `GeminiPartText { part_index: 0 }` need to work.

Return `Err` if the target path doesn't exist in the JSON (signals extract/apply mismatch).

### 4. Update `ParsedFrame`

Replace:
```rust
struct ParsedFrame {
    is_text: bool,
    json: Option<serde_json::Value>,
    raw: String,
}
```

With:
```rust
struct ParsedFrame {
    fields: Vec<FrameFieldContribution>,
    json: Option<serde_json::Value>,
    raw: String,
}
```

### 5. Update frame parsing in `restore_sse`

- Extract `event_type` from each frame: `let event_type = frame.lines().find_map(|l| l.strip_prefix("event: "));`
- Call `extract_fields(&json, provider, event_type)` instead of `extract_text_field`.
- Replace `text_buf: String` with `accumulators: HashMap<FieldKey, String>`.
- For each `ExtractedField` returned, push text into `accumulators.entry(field.key.clone()).or_default()` and record a `FrameFieldContribution { key, byte_len: text.len(), write_back }` in the frame.

### 6. Replace single `restore_stream` with per-buffer sync `doppel::restore`

Replace:
```rust
let text_stream = ...;
let us = restore_stream(text_stream, entries, session_key)...;
```

With:
```rust
let mut restored_buffers: HashMap<FieldKey, String> = HashMap::new();
for (key, buf) in &accumulators {
    let mut input = std::io::Cursor::new(buf.as_bytes());
    let mut output = Vec::new();
    doppel::restore(&mut input, &mut output, &entries, &session_key)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let restored = String::from_utf8(output)
        .map_err(|e| io::Error::other(format!("restore produced non-UTF8: {e}")))?;
    restored_buffers.insert(key.clone(), restored);
}
```

Add `use doppel::restore;` alongside the existing `use doppel::restore_stream;`. Keep `restore_stream` for `restore_non_sse`.

Also add `use std::collections::HashMap;` if not already present.

### 7. Update redistribution logic

Replace the "first text event gets all text" approach with per-field redistribution:

```rust
let mut cursors: HashMap<FieldKey, usize> = HashMap::new();

for mut frame in parsed {
    if frame.fields.is_empty() {
        queue.push_back(Bytes::from(frame.raw.into_bytes()));
        continue;
    }
    let Some(mut json) = frame.json.take() else {
        queue.push_back(Bytes::from(frame.raw.into_bytes()));
        continue;
    };

    let mut write_pairs: Vec<(WriteBackInfo, String)> = Vec::new();
    for contrib in &frame.fields {
        let offset = cursors.entry(contrib.key.clone()).or_insert(0);
        let buf = &restored_buffers[&contrib.key];
        let slice = &buf[*offset..*offset + contrib.byte_len];
        *offset += contrib.byte_len;
        write_pairs.push((contrib.write_back.clone(), slice.to_owned()));
    }

    apply_restored_fields(&mut json, &write_pairs)
        .map_err(|e| io::Error::other(e))?;

    // Reconstruct frame preserving non-data lines
    let prefix_lines: String = frame.raw.lines()
        .filter(|l| !l.starts_with("data:") && !l.is_empty())
        .map(|l| format!("{l}\n"))
        .collect();
    let reconstructed = format!(
        "{}data: {}\n\n",
        prefix_lines,
        serde_json::to_string(&json).map_err(|e| io::Error::other(e.to_string()))?
    );
    queue.push_back(Bytes::from(reconstructed.into_bytes()));
}
```

### 8. Update empty-accumulator early exit

Replace `if text_buf.is_empty()` with `if accumulators.is_empty()`.

### 9. Update unit tests

Replace the existing `extract_text_field` / `set_text_field` unit tests with equivalents for `extract_fields` / `apply_restored_fields`. Each existing test case must have a corresponding new test:

| Old test | New test | What it checks |
|---|---|---|
| `extract_anthropic_text_delta` | `extract_fields_anthropic_text_delta` | Returns 1 field with key `AnthropicDelta { delta_type: "text_delta", index: 0 }` and text "hello" |
| `extract_anthropic_skips_message_start` | `extract_fields_anthropic_skips_message_start` | Returns empty vec |
| `extract_anthropic_skips_content_block_stop` | `extract_fields_anthropic_skips_content_block_stop` | Returns empty vec |
| `extract_openai_delta_content` | `extract_fields_openai_delta_content` | Returns 1 field with key `OpenAiContent` and text "hi" |
| `extract_openai_skips_null_content` | `extract_fields_openai_skips_null_content` | Returns empty vec |
| `extract_gemini_text` | `extract_fields_gemini_text` | Returns 1 field with key `GeminiText { thought: false }` and text "hi" |
| `extract_gemini_skips_empty_parts` | `extract_fields_gemini_skips_empty_parts` | Returns empty vec |
| `set_anthropic_text_field_replaces` | `apply_fields_anthropic_text` | Round-trip: extract then apply with new text, verify JSON |
| `set_anthropic_returns_false_for_non_text` | (covered by `extract_fields_anthropic_skips_message_start`) | — |
| `set_openai_text_field_replaces` | `apply_fields_openai_content` | Round-trip |
| `set_gemini_text_field_replaces` | `apply_fields_gemini_text` | Round-trip |

The async tests (`sse_restore_stream_passthrough_no_secrets`, `non_sse_passthrough_no_secrets`, `empty_stream_produces_empty_output`, `sse_event_lines_preserved_through_text_frame_reconstruction`) must remain unchanged and pass.

### 10. Delete old functions

Remove `extract_text_field` and `set_text_field` after all callers are migrated. They are `pub` but only used within this module and tests — verify no external callers exist with `grep -rn "extract_text_field\|set_text_field" crates/`.

### 11. Add debug assertion for byte-length invariant

After the per-buffer restore loop, add:
```rust
debug_assert!(
    accumulators.iter().all(|(k, buf)| buf.len() == restored_buffers[k].len()),
    "structural-equivalence invariant violated: accumulated and restored buffer lengths differ"
);
```

## Acceptance Criteria

1. **All existing tests pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_anthropic_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_anthropic_sse_stream_with_event_prefix)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openai_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_gemini_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openrouter_sse_stream)'
   ```

2. **All unit tests in sse_restore.rs pass:**
   ```
   cargo nextest run -p lcp-server --lib -E 'test(sse_restore)'
   ```

3. **No compilation warnings:**
   ```
   cargo clippy -p lcp-server --all-targets -- -D warnings
   ```

4. **Old functions removed:**
   ```
   grep -rn "extract_text_field\|set_text_field" crates/lcp-server/src/ | grep -v "extract_fields\|apply_restored_fields"
   ```
   Must return no results.

5. **Full workspace test suite green:**
   ```
   cargo nextest run --workspace
   ```

## Reviewer Instructions

1. Run all 5 acceptance criteria commands above. All must exit 0.
2. Read `crates/lcp-server/src/ext/sse_restore.rs` and verify:
   - `FieldKey` enum has all variants listed above (even ones not yet used — they're needed for future steps)
   - `extract_fields` returns `Vec<ExtractedField>` (not `Option`)
   - `apply_restored_fields` takes `&[(WriteBackInfo, String)]`
   - `restore_sse` uses `HashMap<FieldKey, String>` for accumulation
   - `restore_sse` calls sync `doppel::restore` (not `restore_stream`) per buffer
   - Redistribution uses per-key cursor with `byte_len` tracking
   - Debug assertion checks accumulated vs restored buffer lengths
3. Verify no external callers of `extract_text_field` / `set_text_field` remain.

## Rollback

```
git checkout HEAD -- crates/lcp-server/src/ext/sse_restore.rs
```
