# Plan: Streaming SSE Restore (Sliding Window)

## Goal

Replace `SseRestoreStream`'s full-buffering state machine (`Collecting→Processing→Emitting→Done`) with a streaming sliding-window approach that emits restored SSE frames incrementally as upstream chunks arrive, satisfying SPEC.md §SSE-Aware Restore: Sliding Window.

## Status

| Step | File | Status |
|---|---|---|
| 0 | [step-00-failing-spec-invariant.md](step-00-failing-spec-invariant.md) | TODO |
| 1 | [step-01-sliding-window-core.md](step-01-sliding-window-core.md) | TODO |
| 2 | [step-02-synthetic-frame-emission.md](step-02-synthetic-frame-emission.md) | TODO |
| 3 | [step-03-eof-flush-and-cleanup.md](step-03-eof-flush-and-cleanup.md) | TODO |
| 4 | [step-04-verify-and-update-spec.md](step-04-verify-and-update-spec.md) | TODO |

## Approach Summary

**State machine:** Replace four states with `Detecting → StreamingSse | CollectingNonSse → Processing → Emitting → Done`. Non-SSE path unchanged.

**Per-FieldKey sliding window:** Each FieldKey maintains a `String` accumulation buffer. When `buffer.len() > max_fake_len`, the safe prefix (`buffer.len() - max_fake_len` bytes) is restored via `doppel::restore` and emitted as a synthetic SSE frame. At EOF, all remaining buffers are flushed.

**Synthetic frames:** Outbound frames carry the same provider-specific JSON delta structure as originals but with restored text. Frame granularity MAY differ from original per SPEC.

**AC rebuild per flush:** Each `doppel::restore` call rebuilds the AC automaton. Acceptable cost (<1μs for typical ≤200-byte fakes, flushes at most once per incoming SSE frame).

**Non-SSE path:** Byte-identical to current behavior. Only the SSE path changes.

## Key Decisions

- **D1:** Use `doppel::restore` (sync, public API) per safe-prefix flush. `process_safe_region` is `pub(crate)` in doppel and unavailable.
- **D2:** Two live SSE states (`Detecting`, `StreamingSse`), not four. `Processing`/`Emitting` retained only for non-SSE.
- **D3:** Per-FieldKey `String` buffer with drain-after-restore (no cursor). Hold window = last `max_fake_len` bytes.
- **D4:** Pass-through frames (no text fields) emitted immediately. Text-bearing frames held until safe prefix can be flushed.
- **D5:** `max_fake_len` computed eagerly in `SseRestoreStream::new`.

## Risks

- **Synthetic frame JSON divergence:** Mitigated by minimal JSON matching provider delta structure. Existing VC-SSE-1..13 tests catch regressions.
- **`\r\n` across chunk boundaries:** Normalize raw_buf after each extend before scanning.
- **Multi-FieldKey flush ordering:** Use deterministic iteration order. Spec permits arbitrary inter-FieldKey ordering.

## Open Decisions

_(none — all resolved during synthesis)_
