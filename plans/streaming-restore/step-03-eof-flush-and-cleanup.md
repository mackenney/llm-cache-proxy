# Step 3: EOF Flush + Cleanup

## Objective

Ensure correct EOF handling, remove dead code from the old full-buffer path, and add integration edge-case tests for sliding-window boundary conditions.

## Prerequisite

Steps 1-2 complete (streaming SSE path functional, synthetic frames correct).

## Part A: EOF Flush Verification

The EOF path in `StreamingSse` must:

1. Process any remaining bytes in `raw_buf` as final frame(s) (`extract_complete_frames` with `eof=true`).
2. Process extracted frames through `process_sse_frame` to accumulate final text.
3. Flush ALL remaining accumulator buffers with `eof=true` (safe_len = buf.len(), no hold window).
4. Transition to `Emitting(output_queue)` if frames remain, or `Done` if empty.

**Verify:** A stream where all text fits within `max_fake_len` (total text < max_fake_len) must still produce correct output at EOF. This is the "everything flushes at EOF" case.

## Part B: Edge-Case Integration Tests

Add to `tests/integration/doppel.rs`:

### Test: `sliding_window_single_frame_stream`

Input: 1 SSE frame whose text is shorter than `max_fake_len`.
Expected: All content emitted at EOF flush. Correct restore. No panic.

### Test: `sliding_window_fake_at_window_boundary`

Construct a fake of length F (`max_fake_len = F`). Send exactly F bytes of text across multiple frames, then 1 more byte. The first byte should be released when accumulation crosses `max_fake_len`. The fake spans the entire hold window and should be detected and restored in the EOF flush. Verify restored output contains the original, not the fake.

### Test: `sliding_window_multi_field_independence`

Two independent FieldKeys (e.g., Anthropic `thinking_delta` index 0 and `text_delta` index 1), each containing a different fake. Each field accumulates independently. Verify both fields are correctly restored without cross-contamination. This is similar to VC-SSE-13 but explicitly exercises the sliding-window per-field buffers.

## Part C: Dead Code Removal

Remove functions that are no longer called after the state machine refactor:

- `restore_sse()` — the full-buffer SSE restore function. All its logic is replaced by the `StreamingSse` poll + helpers.
- `process_buffer()` — the `Collecting→Processing` transition. Replaced by `Detecting→StreamingSse|CollectingNonSse` transitions.
- Any helper functions only called by the above.

**Preserve:**
- `restore_non_sse()` — still called by `CollectingNonSse→Processing`.
- `extract_fields()` — still called by `process_sse_frame`.
- `apply_restored_fields()` — keep if used by any remaining path; remove if fully replaced by `build_synthetic_sse_frame`.
- `is_sse_first_chunk()` — still called by `Detecting`.

**Verification:** After removal, `cargo build` must succeed with no dead-code warnings. Run `cargo clippy` to catch any unused imports or functions.

## Part D: Update Doc Comments

- Remove the "Trade-off: complete buffering is required" comment from `SseRestoreStream` doc.
- Update `SseRestoreStream` doc to describe sliding-window behavior:
  ```
  /// SSE-aware restore stream using a per-field sliding window.
  ///
  /// For SSE responses: parses frames incrementally, accumulates text per
  /// FieldKey, and emits restored synthetic frames as soon as the safe prefix
  /// (buffer minus max_fake_len hold) is available. At EOF, flushes all
  /// remaining buffers.
  ///
  /// For non-SSE responses: buffers all bytes, then runs byte-level restore.
  ```

## Verification

```bash
# All tests pass
cargo nextest run

# New edge-case tests pass
cargo nextest run --test integration -E 'test(sliding_window)'

# No dead code warnings
cargo clippy --workspace --all-targets -- -D warnings

# No unused imports
cargo build --workspace 2>&1 | grep -i "unused"
```

## Exit Criteria

- EOF flush produces correct output for all cases (short stream, boundary, multi-field).
- Dead code removed, no compiler warnings.
- Doc comments updated.
- All tests pass (spec, integration, inline).
- `cargo clippy` clean.
