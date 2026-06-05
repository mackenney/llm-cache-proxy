# Step 05: Gemini New Fields — Multi-Part Text, Code Execution, Function Call Args, Metadata Passthrough

## Context

After step-01, the SSE restore pipeline handles only `candidates[0].content.parts[0].text` for Gemini. This step adds:

- **VC-SSE-9:** `parts[N].text` for all N (not just 0), including thought/answer separation — MUST restore
- **VC-SSE-10:** `parts[N].codeExecutionResult.output` — MUST restore
- **VC-SSE-11:** `parts[N].functionCall.args` string values — MUST restore; non-string values MUST NOT be modified
- **VC-SSE-12:** `thoughtSignature` and `groundingMetadata` — MUST NOT modify (passthrough)

Key design decision: Gemini streaming events typically have one part per event. Thought parts have `thought: true` on the part. Accumulation is keyed by `GeminiText { thought: bool }` (not by `part_index`) to correctly separate thinking content from answer content across events. The `part_index` is stored in `WriteBackInfo` for the write-back path.

## Prerequisites

- **step-01** must be complete. `FieldKey::GeminiText`, `FieldKey::GeminiCodeExecOutput`, `FieldKey::GeminiFuncCallArg`, and their `WriteBackInfo` variants must exist.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — the Gemini match arm in `extract_fields` and `apply_restored_fields`.
- `crates/lcp-server/SPEC.md` §SSE-Aware Restore, Gemini table.
- `tests/integration/doppel.rs` — existing Gemini SSE test.

## Implementation

### 1. Extend `extract_fields` Gemini match arm

Replace the single `parts[0].text` extraction with iteration over all parts:

```rust
Provider::Gemini => {
    let parts = match json.pointer("/candidates/0/content/parts").and_then(|v| v.as_array()) {
        Some(p) => p,
        None => return vec![],
    };

    let mut fields = Vec::new();

    for (i, part) in parts.iter().enumerate() {
        // parts[N].text
        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
            let thought = part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false);
            fields.push(ExtractedField {
                key: FieldKey::GeminiText { thought },
                text: text.to_owned(),
                write_back: WriteBackInfo::GeminiPartText { part_index: i },
            });
        }

        // parts[N].codeExecutionResult.output
        if let Some(output) = part.pointer("/codeExecutionResult/output").and_then(|v| v.as_str()) {
            fields.push(ExtractedField {
                key: FieldKey::GeminiCodeExecOutput,
                text: output.to_owned(),
                write_back: WriteBackInfo::GeminiPartCodeExecOutput { part_index: i },
            });
        }

        // parts[N].functionCall.args — each string value is a separate field
        if let Some(args) = part.pointer("/functionCall/args").and_then(|v| v.as_object()) {
            for (arg_key, arg_val) in args {
                if let Some(s) = arg_val.as_str() {
                    fields.push(ExtractedField {
                        key: FieldKey::GeminiFuncCallArg { arg_key: arg_key.clone() },
                        text: s.to_owned(),
                        write_back: WriteBackInfo::GeminiPartFuncCallArg {
                            part_index: i,
                            arg_key: arg_key.clone(),
                        },
                    });
                }
                // Non-string values (numbers, booleans, objects) are skipped.
            }
        }
    }

    fields
}
```

This naturally handles VC-SSE-12 (passthrough): `thoughtSignature` and `groundingMetadata` are top-level fields, not inside `parts`, so the iteration never touches them. No explicit skip is needed — they're simply not extracted.

### 2. Implement `apply_restored_fields` for new Gemini WriteBackInfo variants

```rust
WriteBackInfo::GeminiPartText { part_index } => {
    let target = json.pointer_mut(&format!("/candidates/0/content/parts/{part_index}/text"))
        .ok_or_else(|| format!("missing candidates[0].content.parts[{part_index}].text"))?;
    *target = Value::String(text);
}
WriteBackInfo::GeminiPartCodeExecOutput { part_index } => {
    let target = json.pointer_mut(&format!("/candidates/0/content/parts/{part_index}/codeExecutionResult/output"))
        .ok_or_else(|| format!("missing parts[{part_index}].codeExecutionResult.output"))?;
    *target = Value::String(text);
}
WriteBackInfo::GeminiPartFuncCallArg { part_index, ref arg_key } => {
    let target = json.pointer_mut(&format!("/candidates/0/content/parts/{part_index}/functionCall/args/{arg_key}"))
        .ok_or_else(|| format!("missing parts[{part_index}].functionCall.args.{arg_key}"))?;
    *target = Value::String(text);
}
```

### 3. Add unit tests

**`extract_fields_gemini_multi_part_text`:**
```rust
#[test]
fn extract_fields_gemini_multi_part_text() {
    let v = json!({
        "candidates": [{"content": {"parts": [
            {"text": "thought", "thought": true},
            {"text": "answer"}
        ]}}]
    });
    let fields = extract_fields(&v, Provider::Gemini, None);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].key, FieldKey::GeminiText { thought: true });
    assert_eq!(fields[0].text, "thought");
    assert_eq!(fields[1].key, FieldKey::GeminiText { thought: false });
    assert_eq!(fields[1].text, "answer");
}
```

**`extract_fields_gemini_code_execution_output`:**
```rust
#[test]
fn extract_fields_gemini_code_execution_output() {
    let v = json!({
        "candidates": [{"content": {"parts": [
            {"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "result: 42\n"}}
        ]}}]
    });
    let fields = extract_fields(&v, Provider::Gemini, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::GeminiCodeExecOutput);
    assert_eq!(fields[0].text, "result: 42\n");
}
```

**`extract_fields_gemini_function_call_args`:**
```rust
#[test]
fn extract_fields_gemini_function_call_args() {
    let v = json!({
        "candidates": [{"content": {"parts": [
            {"functionCall": {"name": "lookup", "args": {"query": "New York", "count": 5}}}
        ]}}]
    });
    let fields = extract_fields(&v, Provider::Gemini, None);
    // Only string values: "query" is extracted, "count" (number) is skipped
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::GeminiFuncCallArg { arg_key: "query".into() });
    assert_eq!(fields[0].text, "New York");
}
```

**`extract_fields_gemini_skips_thought_signature`:**
```rust
#[test]
fn extract_fields_gemini_skips_thought_signature() {
    // thoughtSignature is top-level, not inside parts — must not be extracted
    let v = json!({
        "candidates": [{"content": {"parts": [{"text": "answer"}]}}],
        "thoughtSignature": "base64signature=="
    });
    let fields = extract_fields(&v, Provider::Gemini, None);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].key, FieldKey::GeminiText { thought: false });
    // thoughtSignature is not in the fields list
}
```

**`extract_fields_gemini_skips_grounding_metadata`:**
```rust
#[test]
fn extract_fields_gemini_skips_grounding_metadata() {
    let v = json!({
        "candidates": [{"content": {"parts": [{"text": "answer"}]}}],
        "groundingMetadata": {"searchEntryPoint": {"renderedContent": "<html>"}}
    });
    let fields = extract_fields(&v, Provider::Gemini, None);
    assert_eq!(fields.len(), 1);
    // groundingMetadata is not extracted
}
```

**`apply_fields_gemini_multi_part`:**
```rust
#[test]
fn apply_fields_gemini_multi_part() {
    let mut v = json!({
        "candidates": [{"content": {"parts": [
            {"text": "old_thought", "thought": true},
            {"text": "old_answer"}
        ]}}]
    });
    apply_restored_fields(&mut v, &[
        (WriteBackInfo::GeminiPartText { part_index: 0 }, "new_thought".to_owned()),
        (WriteBackInfo::GeminiPartText { part_index: 1 }, "new_answer".to_owned()),
    ]).unwrap();
    assert_eq!(v["candidates"][0]["content"]["parts"][0]["text"], json!("new_thought"));
    assert_eq!(v["candidates"][0]["content"]["parts"][1]["text"], json!("new_answer"));
}
```

**`apply_fields_gemini_code_exec_output`:**
```rust
#[test]
fn apply_fields_gemini_code_exec_output() {
    let mut v = json!({
        "candidates": [{"content": {"parts": [
            {"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "old"}}
        ]}}]
    });
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::GeminiPartCodeExecOutput { part_index: 0 },
        "restored output".to_owned(),
    )]).unwrap();
    assert_eq!(v["candidates"][0]["content"]["parts"][0]["codeExecutionResult"]["output"], json!("restored output"));
}
```

**`apply_fields_gemini_func_call_arg`:**
```rust
#[test]
fn apply_fields_gemini_func_call_arg() {
    let mut v = json!({
        "candidates": [{"content": {"parts": [
            {"functionCall": {"name": "lookup", "args": {"query": "old", "count": 5}}}
        ]}}]
    });
    apply_restored_fields(&mut v, &[(
        WriteBackInfo::GeminiPartFuncCallArg { part_index: 0, arg_key: "query".into() },
        "restored query".to_owned(),
    )]).unwrap();
    assert_eq!(v["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["query"], json!("restored query"));
    // Non-string value unchanged
    assert_eq!(v["candidates"][0]["content"]["parts"][0]["functionCall"]["args"]["count"], json!(5));
}
```

### 4. Add integration tests

**`restore_gemini_multi_part_text` (VC-SSE-9):**

SSE stream with thought parts and answer parts. The request MUST include `generationConfig.thinkingConfig.includeThoughts: true` (confirmed decision: always include everything). The mock upstream stream is constructed directly — no real request is made in integration tests — so simply build the stream with `thought: true` parts.

Use ONE secret. The fake appears in both thought and answer parts. Since doppel produces the same fake for the same secret, both buffers restore correctly.

Stream structure — 4 thought events, then 4 answer events, each with 1/4 of the fake:
```
data: {"candidates":[{"content":{"parts":[{"text":"<PART1>","thought":true}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART2>","thought":true}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART3>","thought":true}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART4>","thought":true}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART1>"}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART2>"}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART3>"}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART4>"}],"role":"model"}}]}
```

Use `GCP` secret with `patterns::gcp()`. Post to `/gemini/v1/models/gemini-2.5-pro:streamGenerateContent`. Assert original secret appears in response, fake does not.

**`restore_gemini_code_execution_output` (VC-SSE-10):**

Single-event stream (code execution results arrive in one event):
```
data: {"candidates":[{"content":{"parts":[{"codeExecutionResult":{"outcome":"OUTCOME_OK","output":"<FULL_FAKE>"}}],"role":"model"}}]}
```

Use `GCP` secret. Assert original appears, fake does not.

**`restore_gemini_function_call_args` (VC-SSE-11):**

Single-event stream:
```
data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"lookup","args":{"query":"<FULL_FAKE>","count":5}}}],"role":"model"}}]}
```

Use `GCP` secret. Assert:
1. Original secret appears in response
2. Fake does not appear
3. `"count":5` is present unchanged (non-string value unmodified)

**`passthrough_gemini_metadata_fields` (VC-SSE-12):**

Stream with `thoughtSignature` and `groundingMetadata` containing text that matches the fake:
```
data: {"candidates":[{"content":{"parts":[{"text":"<PART1>"}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART2>"}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART3>"}],"role":"model"}}]}

data: {"candidates":[{"content":{"parts":[{"text":"<PART4>"}],"role":"model"}}],"thoughtSignature":"<FULL_FAKE>","groundingMetadata":{"searchEntryPoint":{"renderedContent":"<FULL_FAKE>"}}}
```

Assert:
1. The `text` content is restored (original appears, fake does not in text fields)
2. The `thoughtSignature` value passes through unchanged (fake is still present in thoughtSignature)
3. The `groundingMetadata` value passes through unchanged

To test passthrough, assert that the raw response bytes contain the fake string in the metadata fields. Use a two-pronged assertion: parse the last SSE frame's JSON, check that `thoughtSignature` still equals the fake, and `groundingMetadata.searchEntryPoint.renderedContent` still equals the fake.

## Acceptance Criteria

1. **New unit tests pass:**
   ```
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_gemini_multi_part_text)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_gemini_code_execution_output)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_gemini_function_call_args)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_gemini_skips_thought_signature)'
   cargo nextest run -p lcp-server --lib -E 'test(extract_fields_gemini_skips_grounding_metadata)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_gemini_multi_part)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_gemini_code_exec_output)'
   cargo nextest run -p lcp-server --lib -E 'test(apply_fields_gemini_func_call_arg)'
   ```

2. **New integration tests pass:**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_gemini_multi_part_text)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_gemini_code_execution_output)'
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_gemini_function_call_args)'
   cargo nextest run -p lcp-tests --test integration -E 'test(passthrough_gemini_metadata_fields)'
   ```

3. **Existing Gemini test still passes (regression):**
   ```
   cargo nextest run -p lcp-tests --test integration -E 'test(restore_returns_secret_from_gemini_sse_stream)'
   ```

4. **No clippy warnings:**
   ```
   cargo clippy -p lcp-server --all-targets -- -D warnings
   ```

5. **VCs satisfied:**
   - VC-SSE-9: `restore_gemini_multi_part_text` passes
   - VC-SSE-10: `restore_gemini_code_execution_output` passes
   - VC-SSE-11: `restore_gemini_function_call_args` passes
   - VC-SSE-12: `passthrough_gemini_metadata_fields` passes

## Reviewer Instructions

1. Run all acceptance criteria commands. All must exit 0.
2. Read `extract_fields` Gemini arm — verify it iterates ALL parts (not just `parts[0]`).
3. Verify `thought: bool` on the `GeminiText` key separates thinking from answer content.
4. Verify `functionCall.args` only extracts string values — non-string values are skipped.
5. Verify `thoughtSignature` and `groundingMetadata` are never extracted (passthrough by omission).
6. Verify the `passthrough_gemini_metadata_fields` integration test asserts that metadata fields still contain the fake (not restored).

## Rollback

```
git checkout HEAD -- crates/lcp-server/src/ext/sse_restore.rs tests/integration/doppel.rs
```
