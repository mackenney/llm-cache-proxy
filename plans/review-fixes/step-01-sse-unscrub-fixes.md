# Step 01 — sse_unscrub.rs fixes (B1, I2, S1)

**File:** `crates/lcp-server/src/ext/sse_unscrub.rs`
**Wave:** 0 (no deps)

---

## B1 — Trailing empty line in frame reconstruction (line 405-410)

### Root cause

`str::lines()` on a frame ending with `\n\n` yields a trailing `""` element.
The filter `.filter(|l| !l.starts_with("data:"))` keeps it because `""` does not
start with `"data:"`. The subsequent `.map(|l| format!("{l}\n"))` converts it to
`"\n"`, so `prefix_lines` ends with `"\n\n"` — an SSE frame terminator — before
the `data:` line is appended. Result: an empty event is emitted before the real
event; Anthropic SDK silently drops content.

### Change

**Before (line 408):**
```rust
            .filter(|l| !l.starts_with("data:"))
```

**After:**
```rust
            .filter(|l| !l.starts_with("data:") && !l.is_empty())
```

This filters both the `data:` line and the trailing empty string from `lines()`.

---

## I2 — Empty first chunk latches is_sse to false (line 171)

### Root cause

An upstream that emits an empty byte chunk first causes `is_sse_first_chunk(&[])` to
return `false`. Since `is_sse` starts as `None`, the first non-empty check sets
`is_sse = Some(false)`, permanently disabling the SSE path.

### Change

**Before (line 171):**
```rust
                            if is_sse.is_none() {
                                is_sse = Some(is_sse_first_chunk(&chunk));
                            }
```

**After:**
```rust
                            if is_sse.is_none() && !chunk.is_empty() {
                                is_sse = Some(is_sse_first_chunk(&chunk));
                            }
```

### New unit test

Add a test in the `#[cfg(test)] mod tests` block (after existing `is_sse_rejects_empty`
at ~line 450):

```rust
    #[test]
    fn is_sse_detection_skips_empty_first_chunk() {
        // Regression: an empty leading chunk must not latch is_sse to false.
        // The detection should wait for a non-empty chunk.
        // This test documents the intent; the actual guard is in SseUnscrubStream::poll_next.
        assert!(!is_sse_first_chunk(b""));
        assert!(is_sse_first_chunk(b"data: "));
        assert!(is_sse_first_chunk(b"event: "));
    }
```

---

## S1 — Avoid clone of restored_text (line 392)

### Root cause

`restored_text.clone()` allocates an O(n) copy. The original is never read again
after the first text event (subsequent events use `String::new()`).

### Change

**Before (lines 372, 390-392):**
```rust
    let restored_text = String::from_utf8(restored_bytes)
        .map_err(|e| io::Error::other(format!("unscrub_stream produced non-UTF8 bytes: {e}")))?;
```
…
```rust
        let new_text = if !first_text_done {
            first_text_done = true;
            restored_text.clone()
```

**After:**
```rust
    let mut restored_text = String::from_utf8(restored_bytes)
        .map_err(|e| io::Error::other(format!("unscrub_stream produced non-UTF8 bytes: {e}")))?;
```
…
```rust
        let new_text = if !first_text_done {
            first_text_done = true;
            std::mem::take(&mut restored_text)
```

---

## Acceptance

```bash
cargo nextest run -p lcp-server --lib -- sse_unscrub
cargo clippy --workspace --all-targets -- -D warnings
```

All existing `sse_unscrub` tests pass. No new test failures.
