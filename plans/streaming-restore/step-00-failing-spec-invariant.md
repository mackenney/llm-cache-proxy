# Step 0: Write Failing Spec Invariant Tests

## Objective

Create `tests/spec/sse_restore_streaming.rs` with spec invariant tests that enforce SPEC.md §SSE-Aware Restore: Sliding Window. These tests MUST fail against the current full-buffering implementation.

## Prerequisite

None — this is the first step.

## Deliverables

1. `tests/spec/sse_restore_streaming.rs` — new test file
2. `tests/spec/mod.rs` — add `mod sse_restore_streaming;`

## Test Design

### Approach: Direct `SseRestoreStream` construction with channel-backed stream

`SseRestoreStream` is `pub` in `lcp_server`. Construct it directly with a controlled input stream backed by `tokio::sync::mpsc`. This avoids needing a full proxy harness and enables precise control over when input frames arrive vs when output is checked.

**Channel-to-Stream adapter:** Wrap an `mpsc::Receiver<Result<Bytes, io::Error>>` as a `futures::Stream` using `tokio_stream::wrappers::ReceiverStream` (or a manual `poll_fn` adapter). Each `send` on the channel yields one SSE frame chunk.

### Test 1: `inv_sse_streaming_emits_before_eof`

The core sliding-window invariant from SPEC.md: "Phase 3 MUST NOT buffer the entire response before emitting output."

**Setup:**
1. Use `doppel::swap` to produce `entries` for a known secret (e.g., `"sk-secret-key-value-here"`). Record `max_fake_len = entries.iter().map(|e| e.fake.len()).max().unwrap_or(0)`.
2. Create an `mpsc::channel(1)` for input frames.
3. Construct Anthropic-style `text_delta` SSE frames, each carrying 1 byte of text (e.g., `"a"`, `"b"`, ...). These are valid SSE frames: `event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}\n\n`
4. Wrap the receiver as a `ResponseStream` and pass to `SseRestoreStream::new(stream, entries, session_key, Provider::Anthropic)`.

**Execution:**
1. Send `max_fake_len + 5` frames through the channel (do NOT close/drop the sender).
2. After sending, poll the `SseRestoreStream` output. Use `tokio::time::timeout(Duration::from_secs(5), output.next())` or equivalent.
3. **Assert:** At least one `Some(Ok(_))` is received from the output before the sender is dropped. With the current full-buffer implementation, the stream will return `Poll::Pending` indefinitely (it waits for EOF), causing a timeout → test failure.

**Expected behavior:**
- Current code (full buffer): FAIL — no output until EOF, timeout hit.
- Correct sliding-window: PASS — output emitted after `max_fake_len` bytes accumulated.

### Test 2: `inv_sse_streaming_no_entries_passthrough_immediate`

When `entries` is empty, `max_fake_len = 0`. Every frame should pass through immediately with zero hold.

**Setup:** Same as Test 1 but with `entries = vec![]`.

**Execution:**
1. Send 3 SSE frames.
2. After each send, poll output and assert a frame is available immediately (within 1 second timeout).

**Expected behavior:**
- Current code: FAIL — buffers everything even with no entries.
- Correct implementation: PASS — immediate passthrough.

### Test 3: `inv_sse_streaming_correctness_after_window`

End-to-end correctness: restored output must contain the original secret and not the fake.

**Setup:** Same as Test 1.

**Execution:**
1. Build frames that collectively spell out the fake text (character by character), followed by additional plaintext.
2. Drop the sender (signal EOF).
3. Collect all output bytes from `SseRestoreStream`.
4. Parse the output, extract all `delta.text` values, concatenate.
5. Assert: concatenated text contains the original secret, does NOT contain the fake.

**Expected behavior:**
- Current code: FAIL (it will buffer everything but the test verifies streaming happened before EOF via timing assertion similar to Test 1, AND then checks correctness).
- Actually, this test should pass with both implementations for the correctness part. The streaming aspect is covered by Test 1. Make this test focus purely on correctness — it verifies the sliding window doesn't break the restore.
- To make it fail against current code: skip the correctness-only version and instead combine with the before-EOF assertion. OR: simply write it as a correctness test that will pass on both implementations (it still adds value by catching regressions in the new code).

**Decision:** Write this as a correctness test that passes on both old and new code. It serves as a regression guard for the sliding-window implementation, not as a failing-first test. Tests 1 and 2 are the ones that fail first.

## Implementation Notes

- Import `lcp_server::ext::sse_restore::SseRestoreStream` — the type is `pub` in its module but has no `pub use` re-export at the crate root or `ext/mod.rs`. Use the full module path.
- Import `doppel::{swap, Entry, SessionKey}` for creating test entries.
- Use `Provider::Anthropic` for simplicity (well-understood SSE format).
- Frame construction helper: write a local `fn anthropic_text_delta_frame(index: u32, text: &str) -> Bytes` that returns a complete SSE frame as `Bytes`.
- The `ResponseStream` type is `Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>`. Construct from the channel receiver.

## Verification

```bash
# Tests should compile
cargo nextest run --test spec -E 'test(sse_restore_streaming)' --no-run

# Tests 1 and 2 should FAIL (timeout or no output before EOF)
cargo nextest run --test spec -E 'test(inv_sse_streaming_emits_before_eof)' 2>&1 | grep -E 'FAIL|PASS'
cargo nextest run --test spec -E 'test(inv_sse_streaming_no_entries_passthrough_immediate)' 2>&1 | grep -E 'FAIL|PASS'

# Test 3 (correctness) should PASS on current code
cargo nextest run --test spec -E 'test(inv_sse_streaming_correctness_after_window)' 2>&1 | grep -E 'FAIL|PASS'
```

## Exit Criteria

- All three tests compile and run.
- Tests 1 and 2 fail against the current full-buffering implementation.
- Test 3 passes (correctness guard).
- No other tests broken.
