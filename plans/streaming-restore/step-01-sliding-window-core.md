# Step 1: Refactor SseRestoreStream State Machine + Sliding Window Core

## Objective

Replace the `Collecting→Processing→Emitting→Done` state machine with `Detecting→StreamingSse|CollectingNonSse→Processing→Emitting→Done`. Implement the per-FieldKey sliding-window accumulation and safe-prefix flush logic for the SSE path.

## Prerequisite

Step 0 complete (failing spec invariant tests exist).

## Scope

This step implements the core state machine and flush logic. Synthetic frame emission (building the outbound SSE frames from restored text) is the focus of Step 2. In this step, flushed text can be emitted as raw `data:` lines or a minimal placeholder — the important thing is that the sliding window is wired up and output flows before EOF.

However, to keep the implementation self-contained and testable, this step SHOULD implement enough of the synthetic frame builder to produce valid SSE frames. If the full builder is complex, emit a simple `data: {"delta":{"text":"<restored>"}}\n\n` format and refine in Step 2.

## File Changes

**Primary file:** `crates/lcp-server/src/ext/sse_restore.rs`

### 1. Add `max_fake_len` field to `SseRestoreStream`

```rust
pub struct SseRestoreStream {
    state: SseState,
    max_fake_len: usize,
}
```

Compute in `new()`: `entries.iter().map(|e| e.fake.len()).max().unwrap_or(0)`.

### 2. Replace `SseState` enum

```rust
enum SseState {
    Detecting {
        inner: ResponseStream,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
    },
    StreamingSse {
        inner: ResponseStream,
        raw_buf: Vec<u8>,
        accumulators: HashMap<FieldKey, String>,
        entries: Vec<Entry>,
        session_key: SessionKey,
        provider: Provider,
        output_queue: VecDeque<Bytes>,
    },
    CollectingNonSse {
        inner: ResponseStream,
        raw_buf: Vec<u8>,
        entries: Vec<Entry>,
        session_key: SessionKey,
    },
    Processing(BoxFuture<'static, Result<VecDeque<Bytes>, io::Error>>),
    Emitting(VecDeque<Bytes>),
    Done,
}
```

### 3. `Detecting` poll logic

On first non-empty chunk:
- `is_sse_first_chunk(&chunk)` → `true`: transition to `StreamingSse`, push chunk into `raw_buf`.
- `false`: transition to `CollectingNonSse`, push chunk into `raw_buf`.
- Empty chunks: store and keep polling (or skip — match current behavior).
- EOF with no data: transition to `Done`.

### 4. `StreamingSse` poll logic

```
loop {
    // 1. Drain output_queue
    if let Some(bytes) = output_queue.pop_front() {
        self.state = StreamingSse { ... };
        return Poll::Ready(Some(Ok(bytes)));
    }

    // 2. Poll inner stream
    match inner.poll_next(cx) {
        Ready(Some(Ok(chunk))) => {
            raw_buf.extend_from_slice(&chunk);
            // Extract complete SSE frames (terminated by \n\n)
            let frames = extract_complete_frames(&mut raw_buf, false);
            for frame_str in frames {
                process_sse_frame(
                    &frame_str, provider, &mut accumulators, &mut output_queue
                );
            }
            // Flush safe prefixes from all accumulators
            let flushed = flush_safe_prefix(
                &mut accumulators, max_fake_len, &entries, &session_key, provider, false
            )?;
            output_queue.extend(flushed);
            continue; // loop to drain
        }
        Ready(Some(Err(e))) => return Ready(Some(Err(...))),
        Ready(None) => {
            // EOF
            let frames = extract_complete_frames(&mut raw_buf, true);
            for frame_str in frames {
                process_sse_frame(...);
            }
            let flushed = flush_safe_prefix(..., eof=true)?;
            output_queue.extend(flushed);
            if output_queue.is_empty() {
                self.state = Done;
                return Ready(None);
            }
            self.state = Emitting(output_queue);
            continue;
        }
        Pending => {
            self.state = StreamingSse { ... };
            return Pending;
        }
    }
}
```

### 5. `CollectingNonSse` poll logic

Identical to current `Collecting` behavior for non-SSE: buffer all bytes, on EOF transition to `Processing` with `restore_non_sse`. This path is byte-identical to current behavior.

### 6. New helper: `extract_complete_frames`

```rust
fn extract_complete_frames(buf: &mut Vec<u8>, eof: bool) -> Vec<String> {
    // Normalize \r\n → \n in the buffer
    // Split on \n\n delimiter
    // Return complete frames, leave partial trailing frame in buf
    // If eof, include trailing content as final frame
}
```

### 7. New helper: `process_sse_frame`

For each complete SSE frame:
1. Parse `event:` and `data:` lines.
2. Extract text fields using existing `extract_fields()`.
3. If no text fields extracted → push raw frame bytes directly to `output_queue` (pass-through).
4. If text fields extracted → append each field's text to the corresponding `accumulators[field_key]` entry. Do NOT emit the original frame.

### 8. New helper: `flush_safe_prefix`

```rust
fn flush_safe_prefix(
    accumulators: &mut HashMap<FieldKey, String>,
    max_fake_len: usize,
    entries: &[Entry],
    session_key: &SessionKey,
    provider: Provider,
    eof: bool,
) -> Result<Vec<Bytes>, io::Error> {
    for (key, buf) in accumulators.iter_mut() {
        let safe_len = if eof { buf.len() }
                       else if buf.len() > max_fake_len { buf.len() - max_fake_len }
                       else { continue };
        if safe_len == 0 { continue; }

        let safe_prefix = &buf[..safe_len];
        // doppel::restore on safe_prefix
        let restored = restore_text(safe_prefix, entries, session_key)?;
        buf.drain(..safe_len);

        // Build synthetic SSE frame with restored text
        let frame = build_synthetic_sse_frame(key, &restored, provider);
        output.push(Bytes::from(frame));
    }
    Ok(output)
}
```

The `restore_text` helper wraps `doppel::restore` with `Cursor` I/O, matching the existing pattern at line ~808-814 of `restore_sse`.

### 9. Update `SseRestoreStream::new`

Initialize with `Detecting` state. Store `max_fake_len`.

## Existing functions preserved

- `extract_fields()` — unchanged, used by `process_sse_frame`.
- `apply_restored_fields()` — may be used by `build_synthetic_sse_frame` or replaced.
- `is_sse_first_chunk()` — unchanged.
- `restore_non_sse()` — unchanged, used by `CollectingNonSse→Processing` transition.

## Existing functions removed

- `restore_sse()` (full-buffer SSE restore) — replaced by streaming logic.
- `process_buffer()` (the `Collecting→Processing` transition helper) — replaced by split `Detecting` and `CollectingNonSse` transitions.

## `\r\n` handling

Normalize `\r\n → \n` in the raw_buf after each extend, before scanning for `\n\n`. Apply normalization to the full buffer (idempotent, safe to re-apply).

## Verification

```bash
# Must compile
cargo build --workspace

# Spec invariant tests from Step 0 should now PASS (tests 1 and 2)
cargo nextest run --test spec -E 'test(sse_restore_streaming)'

# Existing VC-SSE-1..13 integration tests must still pass
cargo nextest run --test integration

# All inline unit tests pass
cargo nextest run -p lcp-server

# No clippy warnings
cargo clippy --workspace --all-targets -- -D warnings
```

## Exit Criteria

- `inv_sse_streaming_emits_before_eof` passes.
- `inv_sse_streaming_no_entries_passthrough_immediate` passes.
- All existing tests pass (VC-SSE-1..13, inline unit tests, other spec tests).
- `cargo clippy` clean.
