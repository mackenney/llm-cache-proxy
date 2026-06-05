# Step 02: Anthropic New Fields — thinking_delta, input_json_delta, signature_delta

## Context

After the Wave 0 refactor (step-01), the SSE restore pipeline has a multi-buffer architecture but only extracts `text_delta` → `delta.text` for Anthropic. This step adds support for:

- **VC-SSE-1:** `thinking_delta` → `delta.thinking` — MUST restore
- **VC-SSE-2:** `input_json_delta` → `delta.partial_json` — MUST restore
- **VC-SSE-3:** `signature_delta` → `delta.signature` — MUST NOT modify (passthrough)

A single Anthropic response may interleave blocks of different delta types. Each content field is accumulated independently, keyed by `(delta_type, index)`.

## Prerequisites

- **step-01** must be complete. The `FieldKey::AnthropicDelta`, `extract_fields`, `apply_restored_fields`, and multi-buffer accumulation must be in place.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — the refactored file from step-01. Locate the `extract_fields` function's Anthropic match arm and `apply_restored_fields`'s `AnthropicDelta` handling.
- `crates/lcp-server/SPEC.md` §SSE-Aware Restore, Anthropic table — the authoritative field list.
- `tests/integration/doppel.rs` — existing Anthropic SSE tests for the integration test pattern.

## Implementation

### 1. Extend `extract_fields` Anthropic match arm

Currently the Anthropic arm only handles `delta.type == "text_delta"`. Extend to:

```rust
Provider::Anthropic => {
    if json["type"].as_str() != Some("content_block_delta") {
        return vec![];
    }
    let index = json["index"].as_u64().unwrap_or(0);
    let delta = &json["delta"];
    let delta_type = match delta["type"].as_str() {
        Some(dt) => dt,
        None => return vec![],
    };
    match delta_type {
        "text_delta" => {
            if let Some(text) = delta["text"].as_str() {
                vec![ExtractedField {
                    key: FieldKey::AnthropicDelta { delta_type: "text_delta".into(), index },
                    text: text.to_owned(),
                    write_back: WriteBackInfo::AnthropicDelta { field_name: "text".into() },
                }]
            } else { vec![] }
        }
        "thinking_delta" => {
            if let Some(text) = delta["thinking"].as_str() {
                vec![ExtractedField {
                    key: FieldKey::AnthropicDelta { delta_type: "thinking_delta".into(), index },
                    text: text.to_owned(),
                    write_back: WriteBackInfo::AnthropicDelta { field_name: "thinking".into() },
                }]
            } else { vec![] }
        }
        "input_json_delta" => {
            if let Some(text) = delta["partial_json"].as_str() {
                vec![ExtractedField {
                    key: FieldKey::AnthropicDelta { delta_type: "input_json_delta".into(), index },
                    text: text.to_owned(),
                    write_back: WriteBackInfo::AnthropicDelta { field_name: "partial_json".into() },
                }]
            } else { vec![] }
        }
        "signature_delta" => {
            // MUST NOT modify — return empty to skip accumulation entirely.
            vec![]
        }
        _ => vec![],
    }
}
```

### 2. Extend `apply_restored_fields` AnthropicDelta handling

The `WriteBackInfo::AnthropicDelta { field_name }` write-back must set `json["delta"][field_name]`. The existing handler for `field_name: "text"` already works generically — verify it handles `"thinking"` and `"partial_json"` too. The logic should be:

```rust
WriteBackInfo::AnthropicDelta { field_name } => {
    let target = json.get_mut("delta")
        .and_then(|d| d.get_mut(field_name.as_str()))
        .ok_or_else(|| format!("missing delta.{field_name}"))?;
    *target = Value::String(text);
}
```

This is generic over `field_name` — no new code needed if step-01 implemented it this way. If step-01 hardcoded `"text"`, change to use `field_name`.

### 3. Add unit tests

Add to the `#[cfg(test)] mod tests` in `sse_restore.rs`:

**`extract_fields_anthropic_thinking_delta`:**
```rust
#[test]
fn extract_fields_anthropic_thinking_delta() {
    let v = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": { "type": "thinking_delta", "thinking": "Let me reason..." }
    });
    let fields = extract_fields(&v, Provider::Anthropic, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::AnthropicDelta {
        delta_type: "thinking_delta".into(), index: 0
    });
    assert_eq!(fields[0].text, "Let me reason...");
}
```

**`extract_fields_anthropic_input_json_delta`:**
```rust
#[test]
fn extract_fields_anthropic_input_json_delta() {
    let v = json!({
        "type": "content_block_delta",
        "index": 1,
        "delta": { "type": "input_json_delta", "partial_json": "{\"key\":" }
    });
    let fields = extract_fields(&v, Provider::Anthropic, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::AnthropicDelta {
        delta_type: "input_json_delta".into(), index: 1
    });
    assert_eq!(fields[0].text, "{\"key\":");
}
```

**`extract_fields_anthropic_skips_signature_delta`:**
```rust
#[test]
fn extract_fields_anthropic_skips_signature_delta() {
    let v = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": { "type": "signature_delta", "signature": "bWVzc2FnZV9zaWduYXR1cmU=" }
    });
    let fields = extract_fields(&v, Provider::Anthropic, None);
    assert!(fields.is_empty(), "signature_delta MUST be passed through unmodified");
}
```

**`apply_fields_anthropic_thinking`:**
```rust
#[test]
fn apply_fields_anthropic_thinking() {
    let mut v = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": { "type": "thinking_delta", "thinking": "old" }
    });
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::AnthropicDelta { field_name: "thinking".into() },
        "new thought".to_owned(),
    )]).unwrap();
    assert_eq!(v["delta"]["thinking"], json!("new thought"));
}
```

**`apply_fields_anthropic_input_json`:**
```rust
#[test]
fn apply_fields_anthropic_input_json() {
    let mut v = json!({
        "type": "content_block_delta",
        "index": 1,
        "delta": { "type": "input_json_delta", "partial_json": "old" }
    });
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::AnthropicDelta { field_name: "partial_json".into() },
        "{\"restored\":true}".to_owned(),
    )]).unwrap();
    assert_eq!(v["delta"]["partial_json"], json!("{\"restored\":true}"));
}
```

### 4. Add integration tests

Add to `tests/integration/doppel.rs`, following the existing Anthropic SSE test pattern. Each test: pre-compute a fake via `doppel::swap`, split into 4 parts, build synthetic SSE stream, run through `TestHarness`, assert `assert_present(original)` + `assert_absent(fake)`.

**`restore_anthropic_thinking_delta` (VC-SSE-1):**

SSE stream structure:
```
event: message_start
data: {"type":"message_start","message":{...}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"<PART1>"}}

(3 more thinking_delta events with PART2..PART4)

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"The answer is 42."}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"}}

event: message_stop
data: {"type":"message_stop"}
```

Use `ANT` secret with `patterns::anthropic()`. Post to `/anthropic/v1/messages`. Assert the original secret appears in the response and the fake does not.

**`restore_anthropic_input_json_delta` (VC-SSE-2):**

SSE stream structure (tool use response):
```
event: message_start
data: {"type":"message_start","message":{...}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_secret","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"<PART1>"}}

(3 more input_json_delta events with PART2..PART4)

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}

event: message_stop
data: {"type":"message_stop"}
```

Use `ANT` secret with `patterns::anthropic()`. Post to `/anthropic/v1/messages`.

**`passthrough_anthropic_signature_delta` (VC-SSE-3):**

Build a stream containing a `signature_delta` event whose `delta.signature` value contains bytes that happen to match the fake. Assert the signature value is byte-for-byte unchanged in the output.

SSE stream:
```
event: content_block_start
data: {"type":"content_block_start","index":2,"content_block":{"type":"signature","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"signature_delta","signature":"<FAKE_BYTES_AS_BASE64>"}}

event: content_block_stop
data: {"type":"content_block_stop","index":2}
```

This is embedded alongside text_delta events (which DO get restored). The test must verify that:
1. The text_delta secret is restored (existing behavior)
2. The signature_delta value passes through unchanged

Use `ANT` secret, swap it, put the fake in both the `text_delta` events AND the `signature_delta` event. Assert the signature field in the output still contains the fake (not the original).

## Acceptance Criteria

1. **New unit tests pass:**
   ```
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_anthropic_thinking_delta)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_anthropic_input_json_delta)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_anthropic_skips_signature_delta)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_anthropic_thinking)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_anthropic_input_json)'
   ```

2. **New integration tests pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_anthropic_thinking_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_anthropic_input_json_delta)'
   cargo nextest run -p lcp-tests --test integration -E 'test(passthrough_anthropic_signature_delta)'
   ```

3. **Existing Anthropic tests still pass (regression):**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_anthropic_sse_stream)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_anthropic_sse_stream_with_event_prefix)'
   ```

4. **No clippy warnings:**
   ```
   cargo clippy -p lcp-server --all-targets -- -D warnings
   ```

5. **VCs satisfied:**
   - VC-SSE-1: `restore_anthropic_thinking_delta` passes
   - VC-SSE-2: `restore_anthropic_input_json_delta` passes
   - VC-SSE-3: `passthrough_anthropic_signature_delta` passes

## Reviewer Instructions

1. Run all acceptance criteria commands. All must exit 0.
2. Read the `extract_fields` Anthropic match arm — verify all 4 delta types are handled (`text_delta`, `thinking_delta`, `input_json_delta`, `signature_delta`).
3. Verify `signature_delta` returns an empty vec (not extracted, not accumulated).
4. Verify the `passthrough_anthropic_signature_delta` integration test puts the fake in the signature field and asserts it passes through unchanged.

## Rollback

```
git checkout HEAD -- crates/lcp-server/src/ext/sse_restore.rs tests/integration/doppel.rs
```
