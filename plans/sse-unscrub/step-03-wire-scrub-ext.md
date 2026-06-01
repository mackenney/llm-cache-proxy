# Step 03: Wire SseUnscrubStream into ScrubExt

## Context

### Overall Objective

Implement SSE-aware unscrubbing so that fake keys distributed token-by-token across
Anthropic/OpenAI/Gemini SSE `data:` events are detected at the text level and replaced
before the response reaches the client or cache.

### Phase Context

`SseUnscrubStream` is fully implemented (step 02). This step replaces the direct
`unscrub_stream` call in `ScrubExt::on_response_stream` with `SseUnscrubStream`, which
handles both SSE and non-SSE paths internally. The existing failing test
`unscrub_restores_secret_from_anthropic_sse_stream` must start passing; all existing tests
must continue to pass.

### This Step

Modify `crates/lcp-server/src/ext/scrub.rs` to:
1. Add import for `SseUnscrubStream` from `crate::ext::sse_unscrub`
2. Change `_ctx: ProxyCtx` to `ctx: ProxyCtx` in `on_response_stream`
3. Replace the `unscrub_stream(stream, entries, session_key)` call with
   `SseUnscrubStream::new(stream, entries, session_key, ctx.provider)`
4. Remove the now-unused `use its_classified::unscrub_stream;` import from `scrub.rs`
   (the import moves to `sse_unscrub.rs`; it was added there in step 02)

## Prerequisites

- Step 02 merged (`SseUnscrubStream` available in `crate::ext::sse_unscrub`)

## Files to Read Before Starting

- `crates/lcp-server/src/ext/scrub.rs` — the file to modify; read in full
- `crates/lcp-server/src/ext/sse_unscrub.rs` — confirm `SseUnscrubStream::new` signature:
  `(stream: ResponseStream, entries: Vec<Entry>, session_key: SessionKey, provider: Provider) -> SseUnscrubStream`
- `tests/integration/scrub.rs` — understand the failing test
  `unscrub_restores_secret_from_anthropic_sse_stream` (lines ~639–727); it must pass after
  this step

## Implementation

### Task 1: Modify `on_response_stream` in `scrub.rs`

The current function (at the time of writing):

```rust
fn on_response_stream(
    &self,
    _ctx: ProxyCtx,
    state: SensitiveState,
    stream: ResponseStream,
) -> ResponseStream {
    let Some(entries_json) = state.get("entries").map(|s| s.as_bytes().to_vec()) else {
        return stream;
    };
    let Some(key_hex) = state.get("session_key").map(str::to_owned) else {
        return stream;
    };
    drop(state);

    let entries = match Entry::deserialize_entries(&entries_json) {
        Ok(e) => e,
        Err(e) => return error_stream(format!("entries deserialization failed: {e}")),
    };

    let key_bytes = match decode_key_hex(&key_hex) {
        Ok(b) => b,
        Err(e) => return error_stream(format!("session key decode failed: {e}")),
    };
    let session_key = SessionKey::from_bytes(key_bytes);

    match unscrub_stream(stream, entries, session_key) {
        Ok(us) => Box::pin(us.map(|r| r.map_err(|e| io::Error::other(e.to_string())))),
        Err(e) => error_stream(format!("unscrub_stream construction failed: {e}")),
    }
}
```

Replace with:

```rust
fn on_response_stream(
    &self,
    ctx: ProxyCtx,
    state: SensitiveState,
    stream: ResponseStream,
) -> ResponseStream {
    let Some(entries_json) = state.get("entries").map(|s| s.as_bytes().to_vec()) else {
        return stream;
    };
    let Some(key_hex) = state.get("session_key").map(str::to_owned) else {
        return stream;
    };
    drop(state);

    let entries = match Entry::deserialize_entries(&entries_json) {
        Ok(e) => e,
        Err(e) => return error_stream(format!("entries deserialization failed: {e}")),
    };

    let key_bytes = match decode_key_hex(&key_hex) {
        Ok(b) => b,
        Err(e) => return error_stream(format!("session key decode failed: {e}")),
    };
    let session_key = SessionKey::from_bytes(key_bytes);

    Box::pin(SseUnscrubStream::new(stream, entries, session_key, ctx.provider))
}
```

### Task 2: Update imports in `scrub.rs`

Remove the line:
```rust
use its_classified::{scrub, unscrub_stream};
```

Replace with:
```rust
use its_classified::scrub;
```

(The `unscrub_stream` symbol is now used exclusively inside `sse_unscrub.rs`.)

Add:
```rust
use crate::ext::sse_unscrub::SseUnscrubStream;
```

Place this import alongside the other `use crate::...` lines (after `use its_classified::...`).

### Task 3: Verify no other usages of `unscrub_stream` remain in `scrub.rs`

Run `grep -n 'unscrub_stream' crates/lcp-server/src/ext/scrub.rs` — the result must be empty.

### Task 4: Update the inline unit test `phase3_restores_secret_in_response` (if needed)

The existing test at line ~258 in `scrub.rs` tests Phase 3 with a non-SSE response
(a single-chunk JSON body echoing the fake). This test must continue to pass because
`SseUnscrubStream` routes non-SSE streams through `unscrub_non_sse`. No changes expected,
but verify.

## Acceptance Criteria

- [ ] `cargo nextest run -p lcp-server` exits 0 (all unit tests pass)
- [ ] `cargo nextest run --test integration scrub` exits 0 — specifically including
  `integration::scrub::unscrub_restores_secret_from_anthropic_sse_stream`
- [ ] `cargo nextest run --test integration` exits 0 (no regressions)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `grep -c 'unscrub_stream' crates/lcp-server/src/ext/scrub.rs` outputs `0`
- [ ] `grep -c 'SseUnscrubStream' crates/lcp-server/src/ext/scrub.rs` outputs `1`

## Reviewer Instructions

```bash
cd /home/ignacio/pr/llm-cache-proxy

# All unit tests must pass
cargo nextest run -p lcp-server --lib 2>&1 | grep -E 'FAILED|^test result'

# The formerly-failing SSE integration test must now pass
cargo nextest run --test integration -- scrub::unscrub_restores_secret_from_anthropic_sse_stream \
    2>&1 | tail -5

# All integration tests must pass (no regressions)
cargo nextest run --test integration 2>&1 | grep -E 'FAILED|^test result'

# Clippy clean
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | grep error | head -5

# Verify the import swap
grep -n 'unscrub_stream\|SseUnscrubStream' crates/lcp-server/src/ext/scrub.rs
```

Expected:
- Unit tests: `test result: ok`
- `unscrub_restores_secret_from_anthropic_sse_stream`: PASSED
- Integration tests: no FAILED lines
- Clippy: no errors
- `grep` output: `SseUnscrubStream` appears, `unscrub_stream` does NOT appear

## Rollback

`git checkout -- crates/lcp-server/src/ext/scrub.rs`
