# Step 01: SSE Helpers — Detection and Provider Text-Field Functions

## Context

### Overall Objective

Implement SSE-aware unscrubbing so that fake keys distributed token-by-token across
Anthropic/OpenAI/Gemini SSE `data:` events are detected at the text level and replaced
before the response reaches the client or cache.

### Phase Context

This is the foundation step. The helper functions created here are pure, stateless, and
independently testable. Subsequent steps build the stream machinery on top of them.

### This Step

Create `crates/lcp-server/src/ext/sse_unscrub.rs` with three public functions:
`is_sse_first_chunk` (stream-type detection), `extract_text_field` (reads provider-specific
text from a parsed SSE event JSON), and `set_text_field` (writes back a new text value into
the same JSON). Add a `pub mod sse_unscrub;` declaration in `ext/mod.rs`. All functions must
have thorough `#[cfg(test)]` unit tests covering all four providers.

## Prerequisites

- Step-01 has no code prerequisites. Work can begin immediately.
- Confirm `serde_json` is in `crates/lcp-server/Cargo.toml` (it is — verified).

## Files to Read Before Starting

- `crates/lcp-server/src/ext/mod.rs` — add `pub mod sse_unscrub;` here
- `crates/lcp-server/src/ext/scrub.rs` — understand existing patterns for imports and error handling
- `crates/lcp-server/Cargo.toml` — verify `serde_json` dependency (already present)
- `crates/lcp-server/SPEC.md §SSE-Aware Unscrubbing` — provider text-field paths (authoritative)
- `crates/lcp-core/src/lib.rs` (or wherever `Provider` enum lives) — enum variants

## Implementation

### Task 1: Create `crates/lcp-server/src/ext/sse_unscrub.rs`

Add the following at the top of the file (module doc + imports):

```rust
//! SSE-aware unscrubbing helpers for `ScrubExt`.
//!
//! These functions are the building blocks for semantic-level secret
//! restoration in `text/event-stream` responses, where raw-byte Aho-Corasick
//! fails because the fake key never appears contiguously in the byte stream.

use lcp_core::Provider;
use serde_json::Value;
```

> **Note:** The `Provider` enum variant for OpenAI is `Provider::OpenAi` (not `Provider::OpenAI`).
> Use `Provider::OpenAi` in all match arms — including for the "OpenAI / OpenRouter" cases below.

**Function 1 — `is_sse_first_chunk`**

```rust
/// Returns `true` if the first bytes of a response chunk look like an SSE stream.
///
/// The heuristic: SSE streams from all supported providers begin with `data: `.
/// Non-SSE JSON responses begin with `{`. No content-type header access is needed.
pub fn is_sse_first_chunk(bytes: &[u8]) -> bool {
    bytes.starts_with(b"data: ") || bytes.starts_with(b"data:{")
}
```

Note: `data:{` handles the (uncommon) case of no space after the colon.

**Function 2 — `extract_text_field`**

Signature: `pub fn extract_text_field<'v>(json: &'v Value, provider: Provider) -> Option<&'v str>`

Logic per provider (use `serde_json::Value::pointer` for nested access):

- **Anthropic**: Only when `json["type"] == "content_block_delta"` AND
  `json["delta"]["type"] == "text_delta"`. Return `json["delta"]["text"].as_str()`.
- **OpenAI / OpenRouter**: Return `json.pointer("/choices/0/delta/content")?.as_str()`.
  There is NO event-type guard needed — the field is simply absent on non-text events.
- **Gemini**: Return `json.pointer("/candidates/0/content/parts/0/text")?.as_str()`.

Return `None` when the event does not carry text content (non-text events, or fields absent).

**Function 3 — `set_text_field`**

Signature: `pub fn set_text_field(json: &mut Value, provider: Provider, text: String) -> bool`

Returns `true` if the field was located and set, `false` if not found (non-text event).

Logic per provider:

- **Anthropic**: Navigate to `json["delta"]["text"]`. If it exists, set to
  `Value::String(text)`, return `true`. Otherwise `false`.
- **OpenAI / OpenRouter**: Navigate to `json["choices"][0]["delta"]["content"]`. If present,
  set and return `true`. Otherwise `false`.
- **Gemini**: Navigate to `json["candidates"][0]["content"]["parts"][0]["text"]`. If present,
  set and return `true`. Otherwise `false`.

Use `Value::get_mut` chains for navigation. Do NOT use pointer-mutation helpers (not stable
in serde_json 1.x); navigate step by step.

### Task 2: Update `crates/lcp-server/src/ext/mod.rs`

Add before the existing `pub mod scrub;` line:

```rust
pub mod sse_unscrub;
```

The module is `pub` for intra-crate use but is not re-exported from the crate's public API
(keep `pub use scrub::{ScrubExt, ScrubExtLoadError};` unchanged).

### Task 3: Unit tests in `sse_unscrub.rs`

Add a `#[cfg(test)] mod tests` block with the following cases:

| Test name | What it verifies |
|-----------|-----------------|
| `is_sse_detects_data_prefix` | `b"data: {\"type\":\"message_start\"}\n\n"` → `true` |
| `is_sse_rejects_json` | `b"{\"type\":\"message\"}"` → `false` |
| `is_sse_rejects_empty` | `b""` → `false` |
| `extract_anthropic_text_delta` | type=content_block_delta, delta.type=text_delta → `Some("hello")` |
| `extract_anthropic_skips_message_start` | type=message_start → `None` |
| `extract_anthropic_skips_content_block_stop` | type=content_block_stop → `None` |
| `extract_openai_delta_content` | `{"choices":[{"delta":{"content":"hi"}}]}` → `Some("hi")` |
| `extract_openai_skips_null_content` | `{"choices":[{"delta":{}}]}` → `None` |
| `extract_gemini_text` | `{"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}` → `Some("hi")` |
| `extract_gemini_skips_empty_parts` | `{"candidates":[{"content":{"parts":[]}}]}` → `None` |
| `set_anthropic_text_field_replaces` | Start with text_delta JSON, call set → json["delta"]["text"] == "new" |
| `set_anthropic_returns_false_for_non_text` | message_start JSON → `false`, JSON unchanged |
| `set_openai_text_field_replaces` | choices[0].delta.content updated |
| `set_gemini_text_field_replaces` | candidates[0].content.parts[0].text updated |

Use inline JSON literals (`serde_json::json!` macro) to build test values.

## Acceptance Criteria

- [ ] `cargo nextest run -p lcp-server --lib 2>&1 | grep -E "^(test|FAILED|error)"` shows all `sse_unscrub::tests::*` passing, no failures
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo build -p lcp-server` exits 0
- [ ] `grep -n 'pub mod sse_unscrub' crates/lcp-server/src/ext/mod.rs` prints a match
- [ ] `grep -c 'fn is_sse_first_chunk' crates/lcp-server/src/ext/sse_unscrub.rs` outputs `1`
- [ ] `grep -c 'fn extract_text_field' crates/lcp-server/src/ext/sse_unscrub.rs` outputs `1`
- [ ] `grep -c 'fn set_text_field' crates/lcp-server/src/ext/sse_unscrub.rs` outputs `1`

## Reviewer Instructions

```bash
cd /home/ignacio/pr/llm-cache-proxy

# Build must succeed
cargo build -p lcp-server 2>&1; echo "exit: $?"

# All unit tests in the new module must pass
cargo nextest run -p lcp-server --lib 2>&1 | grep -E 'sse_unscrub|FAILED|error\[|^test result'

# Clippy must be clean
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5

# Verify module declaration
grep -n 'pub mod sse_unscrub' crates/lcp-server/src/ext/mod.rs

# Verify all three public functions exist
grep -n 'pub fn ' crates/lcp-server/src/ext/sse_unscrub.rs
```

Expected: build exits 0, all `sse_unscrub::tests::*` tests pass, clippy clean, three `pub fn` lines printed.

## Rollback

Delete `crates/lcp-server/src/ext/sse_unscrub.rs` and revert `ext/mod.rs`.
