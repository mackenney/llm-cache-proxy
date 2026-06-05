# Step 03: OpenAI/OpenRouter New Fields — tool_calls, reasoning_content, function_call

## Context

After step-01, the SSE restore pipeline handles `choices[0].delta.content` for OpenAI/OpenRouter. This step adds:

- **VC-SSE-4:** `choices[0].delta.tool_calls[N].function.arguments` — MUST restore, accumulated per tool-call index
- **VC-SSE-5:** `choices[0].delta.reasoning_content` — MUST restore (OpenAI and OpenRouter)
- **VC-SSE-6:** `choices[0].delta.function_call.arguments` — MUST restore (deprecated API)
- **VC-SSE-6b:** `choices[0].delta.refusal` — MUST restore

## Prerequisites

- **step-01** must be complete. `FieldKey::OpenAiToolCall`, `FieldKey::OpenAiReasoning`, `FieldKey::OpenAiFunctionCallArgs`, `FieldKey::OpenAiRefusal`, and the corresponding `WriteBackInfo` variants must exist.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — locate the `extract_fields` OpenAI/OpenRouter match arm and `apply_restored_fields` OpenAI handlers.
- `crates/lcp-server/SPEC.md` §SSE-Aware Restore, OpenAI/OpenRouter table.
- `tests/integration/doppel.rs` — existing OpenAI/OpenRouter SSE tests.

## Implementation

### 1. Extend `extract_fields` OpenAI/OpenRouter match arm

The current arm extracts only `choices[0].delta.content`. Extend to extract multiple fields from the same event. All these fields live under `choices[0].delta`:

```rust
Provider::OpenAi | Provider::OpenRouter => {
    // Responses API is handled separately — check event_type first.
    // (This guard is a no-op until step-04 adds Responses API support;
    // include it now so step-04 only needs to add the else-branch.)
    if event_type.map_or(false, |e| e.starts_with("response.")) {
        return vec![]; // Handled in step-04
    }

    let delta = match json.pointer("/choices/0/delta") {
        Some(d) => d,
        None => return vec![],
    };

    let mut fields = Vec::new();

    // content
    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
        fields.push(ExtractedField {
            key: FieldKey::OpenAiContent,
            text: text.to_owned(),
            write_back: WriteBackInfo::OpenAiContent,
        });
    }

    // reasoning_content
    if let Some(text) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        fields.push(ExtractedField {
            key: FieldKey::OpenAiReasoning,
            text: text.to_owned(),
            write_back: WriteBackInfo::OpenAiReasoning,
        });
    }

    // tool_calls — each element has an `index` identifying the tool call
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for (array_pos, tc) in tool_calls.iter().enumerate() {
            let tc_index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(args) = tc.get("function").and_then(|f| f.get("arguments")).and_then(|a| a.as_str()) {
                if !args.is_empty() {
                    fields.push(ExtractedField {
                        key: FieldKey::OpenAiToolCall { index: tc_index },
                        text: args.to_owned(),
                        write_back: WriteBackInfo::OpenAiToolCall { array_pos },
                    });
                }
            }
        }
    }

    // function_call.arguments (deprecated)
    if let Some(args) = delta.get("function_call").and_then(|f| f.get("arguments")).and_then(|a| a.as_str()) {
        if !args.is_empty() {
            fields.push(ExtractedField {
                key: FieldKey::OpenAiFunctionCallArgs,
                text: args.to_owned(),
                write_back: WriteBackInfo::OpenAiFunctionCallArgs,
            });
        }
    }

    // refusal
    if let Some(text) = delta.get("refusal").and_then(|v| v.as_str()) {
        fields.push(ExtractedField {
            key: FieldKey::OpenAiRefusal,
            text: text.to_owned(),
            write_back: WriteBackInfo::OpenAiRefusal,
        });
    }

    fields
}
```

### 2. Implement `apply_restored_fields` for new WriteBackInfo variants

```rust
WriteBackInfo::OpenAiReasoning => {
    let target = json.pointer_mut("/choices/0/delta/reasoning_content")
        .ok_or("missing choices[0].delta.reasoning_content")?;
    *target = Value::String(text);
}
WriteBackInfo::OpenAiToolCall { array_pos } => {
    let target = json.pointer_mut(&format!("/choices/0/delta/tool_calls/{array_pos}/function/arguments"))
        .ok_or_else(|| format!("missing choices[0].delta.tool_calls/{array_pos}/function/arguments"))?;
    *target = Value::String(text);
}
WriteBackInfo::OpenAiFunctionCallArgs => {
    let target = json.pointer_mut("/choices/0/delta/function_call/arguments")
        .ok_or("missing choices[0].delta.function_call.arguments")?;
    *target = Value::String(text);
}
WriteBackInfo::OpenAiRefusal => {
    let target = json.pointer_mut("/choices/0/delta/refusal")
        .ok_or("missing choices[0].delta.refusal")?;
    *target = Value::String(text);
}
```

### 3. Add unit tests

**`extract_fields_openai_tool_calls`:**
```rust
#[test]
fn extract_fields_openai_tool_calls() {
    let v = json!({
        "choices": [{"index": 0, "delta": {
            "tool_calls": [{"index": 0, "function": {"arguments": "{\"q\":"}}]
        }}]
    });
    let fields = extract_fields(&v, Provider::OpenAi, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::OpenAiToolCall { index: 0 });
    assert_eq!(fields[0].text, "{\"q\":");
}
```

**`extract_fields_openai_tool_calls_skips_empty_arguments`:**
```rust
#[test]
fn extract_fields_openai_tool_calls_skips_empty_arguments() {
    // First tool_calls event has name + empty arguments — must not extract.
    let v = json!({
        "choices": [{"index": 0, "delta": {
            "tool_calls": [{"index": 0, "id": "call_abc", "type": "function",
                            "function": {"name": "get_secret", "arguments": ""}}]
        }}]
    });
    let fields = extract_fields(&v, Provider::OpenAi, None);
    assert!(fields.is_empty());
}
```

**`extract_fields_openai_reasoning_content`:**
```rust
#[test]
fn extract_fields_openai_reasoning_content() {
    let v = json!({
        "choices": [{"index": 0, "delta": {"reasoning_content": "thinking..."}}]
    });
    let fields = extract_fields(&v, Provider::OpenAi, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::OpenAiReasoning);
    assert_eq!(fields[0].text, "thinking...");
}
```

**`extract_fields_openai_function_call_args`:**
```rust
#[test]
fn extract_fields_openai_function_call_args() {
    let v = json!({
        "choices": [{"index": 0, "delta": {
            "function_call": {"arguments": "{\"loc\":"}
        }}]
    });
    let fields = extract_fields(&v, Provider::OpenAi, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::OpenAiFunctionCallArgs);
    assert_eq!(fields[0].text, "{\"loc\":");
}
```

**`extract_fields_openai_refusal`:**

```rust
#[test]
fn extract_fields_openai_refusal() {
    let v = json!({"choices": [{"delta": {"refusal": "I cannot do that"}}]});
    let fields = extract_fields(&v, Provider::OpenAi, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::OpenAiRefusal);
    assert_eq!(fields[0].text, "I cannot do that");
}
```

**`extract_fields_openrouter_reasoning_content`:**
```rust
#[test]
fn extract_fields_openrouter_reasoning_content() {
    let v = json!({
        "choices": [{"index": 0, "delta": {"reasoning_content": "step 1..."}}]
    });
    let fields = extract_fields(&v, Provider::OpenRouter, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::OpenAiReasoning);
}
```

**`apply_fields_openai_tool_calls`:**
```rust
#[test]
fn apply_fields_openai_tool_calls() {
    let mut v = json!({
        "choices": [{"index": 0, "delta": {
            "tool_calls": [{"index": 0, "function": {"arguments": "old"}}]
        }}]
    });
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::OpenAiToolCall { array_pos: 0 },
        "restored".to_owned(),
    )]).unwrap();
    assert_eq!(v["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"], json!("restored"));
}
```

**`apply_fields_openai_reasoning`:**
```rust
#[test]
fn apply_fields_openai_reasoning() {
    let mut v = json!({
        "choices": [{"index": 0, "delta": {"reasoning_content": "old"}}]
    });
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::OpenAiReasoning,
        "restored".to_owned(),
    )]).unwrap();
    assert_eq!(v["choices"][0]["delta"]["reasoning_content"], json!("restored"));
}
```

### 4. Add integration tests

Follow the existing pattern: swap secret, split fake into 4 parts, build SSE stream, run through `TestHarness`.

**`restore_openai_tool_calls_arguments` (VC-SSE-4):**

SSE stream (see test plan Skeleton 3):
```
data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_secret","arguments":""}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"<PART1>"}}]},"finish_reason":null}]}

(3 more events with PART2..PART4)

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

Use `OPENAI_CLASSIC` secret with `patterns::openai_classic()`. Post to `/openai/v1/chat/completions`.

**`restore_openai_reasoning_content` (VC-SSE-5):**

SSE stream:
```
data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"o4-mini","choices":[{"index":0,"delta":{"reasoning_content":"<PART1>"},"finish_reason":null}]}

(3 more events with PART2..PART4)

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"o4-mini","choices":[{"index":0,"delta":{"content":"Final answer."},"finish_reason":null}]}

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"o4-mini","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

Use `OPENAI_CLASSIC` secret. Post to `/openai/v1/chat/completions`.

**`restore_openrouter_reasoning_content` (VC-SSE-5):**

Same SSE stream structure as OpenAI reasoning_content. Post to `/openrouter/v1/chat/completions`.

**`restore_openai_refusal` (VC-SSE-6b):**

SSE stream:
```
data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":null,"refusal":"<PART1>"},"finish_reason":null}]}

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"refusal":"<PART2>"},"finish_reason":null}]}

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"refusal":"<PART3>"},"finish_reason":null}]}

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"refusal":"<PART4>"},"finish_reason":null}]}

data: [DONE]
```

Use `OPENAI_CLASSIC` secret. Post to `/openai/v1/chat/completions`. Assert response `refusal` field contains original secret.

**`restore_openai_deprecated_function_call` (VC-SSE-6):**

SSE stream:
```
data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":null,"function_call":{"name":"get_info","arguments":""}},"finish_reason":null}]}

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"function_call":{"arguments":"<PART1>"}},"finish_reason":null}]}

(3 more events with PART2..PART4)

data: {"id":"chatcmpl-x","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"function_call"}]}

data: [DONE]
```

Use `OPENAI_CLASSIC` secret. Post to `/openai/v1/chat/completions`.

## Acceptance Criteria

1. **New unit tests pass:**
   ```
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_openai_tool_calls)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_openai_reasoning_content)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_openai_function_call_args)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_openrouter_reasoning_content)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_openai_tool_calls)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_openai_reasoning)'
   ```

2. **New integration tests pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_tool_calls_arguments)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_reasoning_content)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openrouter_reasoning_content)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_deprecated_function_call)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_refusal)'
   ```

3. **Existing OpenAI/OpenRouter tests still pass (regression):**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openai_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openrouter_sse_stream)'
   ```

4. **No clippy warnings:**
   ```
   cargo clippy -p lcp-server --all-targets -- -D warnings
   ```

5. **VCs satisfied:**
   - VC-SSE-4: `restore_openai_tool_calls_arguments` passes
   - VC-SSE-5: `restore_openai_reasoning_content` + `restore_openrouter_reasoning_content` pass
   - VC-SSE-6: `restore_openai_deprecated_function_call` passes
   - VC-SSE-6b: `restore_openai_refusal` passes
## Reviewer Instructions

1. Run all acceptance criteria commands. All must exit 0.
2. Read `extract_fields` OpenAI arm — verify `content`, `reasoning_content`, `tool_calls[N].function.arguments`, `function_call.arguments`, and `refusal` are all extracted.
3. Verify tool_calls accumulates by the tool-call `index` field (not array position).
4. Verify the Responses API guard `event_type.starts_with("response.")` is present (returns empty vec for now).
5. Verify empty `arguments: ""` on the first tool_calls event is skipped.

## Rollback

```
git checkout HEAD -- crates/lcp-server/src/ext/sse_restore.rs tests/integration/doppel.rs
```
