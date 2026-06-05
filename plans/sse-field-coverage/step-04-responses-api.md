# Step 04: OpenAI Responses API — output_text and reasoning_summary_text

## Context

The OpenAI Responses API (`v1/responses`) uses a completely different event schema from chat completions. Events have SSE `event:` lines starting with `response.`, and content fields are top-level (`delta`, `text`) rather than nested under `choices`. This step adds:

- **VC-SSE-7:** `response.output_text.delta` → `delta` field, and `response.output_text.done` → `text` field — MUST restore
- **VC-SSE-8:** `response.reasoning_summary_text.delta` → `delta` field — MUST restore

The Responses API is detected by checking the SSE `event:` line value. If it starts with `response.`, the chat completion extraction paths are skipped entirely.

**Design decision:** `done` events use a separate `FieldKey` (`ResponsesApiDone`) from `delta` events (`ResponsesApiDelta`). The `done` event contains the full assembled text, so its single-frame buffer is complete and restored independently. Both produce correct output since doppel restore is idempotent for the same fakes.

## Prerequisites

- **step-01** must be complete. `FieldKey::ResponsesApiDelta`, `FieldKey::ResponsesApiDone`, and their `WriteBackInfo` variants must exist.
- **step-03** should be complete (the Responses API guard in the OpenAI arm was added there). If not, add the guard here.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — the OpenAI/OpenRouter match arm in `extract_fields`. Look for the `response.` guard added in step-03.
- `crates/lcp-server/SPEC.md` §SSE-Aware Restore, OpenAI Responses API table.
- `tests/integration/doppel.rs` — existing OpenAI SSE test pattern.

## Implementation

### 1. Extend `extract_fields` OpenAI arm with Responses API branch

In the `Provider::OpenAi | Provider::OpenRouter` match arm, the `response.` guard should branch to Responses API extraction:

```rust
if let Some(et) = event_type {
    if et.starts_with("response.") {
        return match et {
            "response.output_text.delta" => {
                if let Some(text) = json.get("delta").and_then(|v| v.as_str()) {
                    vec![ExtractedField {
                        key: FieldKey::ResponsesApiDelta { event_type: "output_text".into() },
                        text: text.to_owned(),
                        write_back: WriteBackInfo::ResponsesApiDelta,
                    }]
                } else { vec![] }
            }
            "response.output_text.done" => {
                if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                    vec![ExtractedField {
                        key: FieldKey::ResponsesApiDone { event_type: "output_text".into() },
                        text: text.to_owned(),
                        write_back: WriteBackInfo::ResponsesApiDone,
                    }]
                } else { vec![] }
            }
            "response.reasoning_summary_text.delta" => {
                if let Some(text) = json.get("delta").and_then(|v| v.as_str()) {
                    vec![ExtractedField {
                        key: FieldKey::ResponsesApiDelta { event_type: "reasoning_summary_text".into() },
                        text: text.to_owned(),
                        write_back: WriteBackInfo::ResponsesApiDelta,
                    }]
                } else { vec![] }
            }
            _ => vec![],
        };
    }
}
// ...existing chat completions extraction below...
```

### 2. Implement `apply_restored_fields` for Responses API variants

```rust
WriteBackInfo::ResponsesApiDelta => {
    let target = json.get_mut("delta")
        .ok_or("missing top-level delta")?;
    *target = Value::String(text);
}
WriteBackInfo::ResponsesApiDone => {
    let target = json.get_mut("text")
        .ok_or("missing top-level text")?;
    *target = Value::String(text);
}
```

### 3. Add unit tests

**`extract_fields_responses_api_text_delta`:**
```rust
#[test]
fn extract_fields_responses_api_text_delta() {
    let v = json!({
        "type": "response.output_text.delta",
        "output_index": 0,
        "content_index": 0,
        "delta": "Hello world"
    });
    let fields = extract_fields(&v, Provider::OpenAi, Some("response.output_text.delta"));
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::ResponsesApiDelta { event_type: "output_text".into() });
    assert_eq!(fields[0].text, "Hello world");
}
```

**`extract_fields_responses_api_text_done`:**
```rust
#[test]
fn extract_fields_responses_api_text_done() {
    let v = json!({
        "type": "response.output_text.done",
        "output_index": 0,
        "content_index": 0,
        "text": "Full assembled text here"
    });
    let fields = extract_fields(&v, Provider::OpenAi, Some("response.output_text.done"));
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::ResponsesApiDone { event_type: "output_text".into() });
    assert_eq!(fields[0].text, "Full assembled text here");
}
```

**`extract_fields_responses_api_reasoning_delta`:**
```rust
#[test]
fn extract_fields_responses_api_reasoning_delta() {
    let v = json!({
        "type": "response.reasoning_summary_text.delta",
        "output_index": 0,
        "summary_index": 0,
        "delta": "reasoning step"
    });
    let fields = extract_fields(&v, Provider::OpenAi, Some("response.reasoning_summary_text.delta"));
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::ResponsesApiDelta { event_type: "reasoning_summary_text".into() });
}
```

**`extract_fields_responses_api_skips_non_content_events`:**
```rust
#[test]
fn extract_fields_responses_api_skips_non_content_events() {
    let v = json!({"type": "response.created", "response": {"id": "resp_test"}});
    let fields = extract_fields(&v, Provider::OpenAi, Some("response.created"));
    assert!(fields.is_empty());
}
```

**`extract_fields_responses_api_does_not_use_chat_completions_paths`:**
```rust
#[test]
fn extract_fields_responses_api_does_not_use_chat_completions_paths() {
    // A Responses API event should NOT be parsed with chat completion paths,
    // even if it happened to contain a choices array (it won't, but defense in depth).
    let v = json!({
        "type": "response.output_text.delta",
        "delta": "correct",
        "choices": [{"index": 0, "delta": {"content": "wrong"}}]
    });
    let fields = extract_fields(&v, Provider::OpenAi, Some("response.output_text.delta"));
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].text, "correct");
}
```

**`apply_fields_responses_api_delta`:**
```rust
#[test]
fn apply_fields_responses_api_delta() {
    let mut v = json!({"type": "response.output_text.delta", "delta": "old"});
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::ResponsesApiDelta,
        "restored".to_owned(),
    )]).unwrap();
    assert_eq!(v["delta"], json!("restored"));
}
```

**`apply_fields_responses_api_done`:**
```rust
#[test]
fn apply_fields_responses_api_done() {
    let mut v = json!({"type": "response.output_text.done", "text": "old"});
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::ResponsesApiDone,
        "restored full text".to_owned(),
    )]).unwrap();
    assert_eq!(v["text"], json!("restored full text"));
}
```

### 4. Add integration tests

**`restore_openai_responses_api_text_delta` (VC-SSE-7):**

SSE stream (see test plan Skeleton 2):
```
event: response.created
data: {"type":"response.created","response":{"id":"resp_test","status":"in_progress"}}

event: response.output_item.added
data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}

event: response.content_part.added
data: {"type":"response.content_part.added","output_index":0,"content_index":0,"part":{"type":"output_text","text":""}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"<PART1>"}

(3 more delta events with PART2..PART4)

event: response.output_text.done
data: {"type":"response.output_text.done","output_index":0,"content_index":0,"text":"<FULL_FAKE>"}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_test","status":"completed"}}
```

The `done` event contains the full fake concatenated. The test asserts:
1. Original secret appears in the response
2. Fake does not appear in the response
3. Both the delta events and the done event are restored

Use `OPENAI_CLASSIC` secret. Post to `/openai/v1/responses`.

**`restore_openai_responses_api_reasoning_delta` (VC-SSE-8):**

SSE stream:
```
event: response.reasoning_summary_text.delta
data: {"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"<PART1>"}

(3 more delta events with PART2..PART4)

event: response.output_text.delta
data: {"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"The answer is 42."}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_test","status":"completed"}}
```

Use `OPENAI_CLASSIC` secret. Post to `/openai/v1/responses`.

## Acceptance Criteria

1. **New unit tests pass:**
   ```
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_responses_api)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_responses_api)'
   ```

2. **New integration tests pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_responses_api_text_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_openai_responses_api_reasoning_delta)'
   ```

3. **Existing OpenAI test still passes (regression):**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_openai_sse_stream)'
   ```

4. **No clippy warnings:**
   ```
   cargo clippy -p lcp-server --all-targets -- -D warnings
   ```

5. **VCs satisfied:**
   - VC-SSE-7: `restore_openai_responses_api_text_delta` passes (covers both delta and done events)
   - VC-SSE-8: `restore_openai_responses_api_reasoning_delta` passes

## Reviewer Instructions

1. Run all acceptance criteria commands. All must exit 0.
2. Read the Responses API branch in `extract_fields` — verify it handles `response.output_text.delta`, `response.output_text.done`, and `response.reasoning_summary_text.delta`.
3. Verify `done` events use `ResponsesApiDone` key (separate accumulation from deltas).
4. Verify chat completion paths are NOT reached when `event_type` starts with `response.`.
5. Verify the integration test for VC-SSE-7 includes a `done` event with the full fake.

## Rollback

```
git checkout HEAD -- crates/lcp-server/src/ext/sse_restore.rs tests/integration/doppel.rs
```
