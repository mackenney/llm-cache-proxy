# Step 03: Spec Invariant Tests for VC-SSE-14..20

## Context

### Overall Objective

Fix the terminal-event ordering bug in `SseRestoreStream`: terminal SSE frames
are forwarded on path A of `process_one_frame` before held accumulator content
is flushed, violating SPEC.md §Terminal Event Ordering (VC-SSE-14..20).

### Phase Context

Wave 3 of 3. Steps 01–02 implemented the fix. This step locks the behavior
in as external behavioral contracts: one spec invariant test per VC
(VC-SSE-14 through VC-SSE-20) plus one wire-level integration regression test
reproducing the original bug trigger.

### This Step

- New file `tests/spec/sse_terminal_ordering.rs`, registered in
  `tests/spec/mod.rs`.
- One new test in `tests/integration/doppel.rs`.

## Prerequisites

- Step 02 merged and green (`cargo nextest run` exits 0). These tests assert
  post-fix ordering; they MUST fail if step 02 is reverted (verify per
  Reviewer Instructions).

## Files to Read Before Starting

- `crates/lcp-server/SPEC.md` VC-SSE-14..20 (lines ~590–627) — the exact
  assertions to encode.
- `tests/spec/sse_restore_streaming.rs` — the pattern to copy: synthetic
  doppel key + `doppel::swap` to get `entries`/`session_key`/`max_fake_len`,
  `SseRestoreStream::new(stream, entries, session_key, provider)` over an
  unbounded mpsc channel, frame-builder helpers, timeouts around polls.
- `tests/spec/mod.rs` — module registration.
- `tests/integration/doppel.rs` — `restore_returns_secret_from_anthropic_sse_stream`
  (~line 683) and `restore_anthropic_input_json_delta` (~line 1123): the
  MockUpstream SSE pattern, how the fake is obtained for mock response bodies,
  and the `assert_present`/`assert_absent` helpers.
- `tests/common/mock_upstream.rs` — `MockUpstream::builder().sse(...)`.

## Implementation

### Test harness conventions (spec file)

Create `tests/spec/sse_terminal_ordering.rs` with a module doc comment citing
SPEC.md §Terminal Event Ordering. Shared helpers:

- Reuse the synthetic Anthropic-format key + `doppel::swap` setup from
  `sse_restore_streaming.rs` (NOT a real credential).
- `unbounded_receiver_to_stream` — copy from `sse_restore_streaming.rs`.
- Frame builders per provider (Anthropic `text_delta`/`input_json_delta`
  delta, `content_block_stop`, `message_delta`, `message_stop`; OpenAI chat
  tool-call delta + finish-reason frame + `[DONE]`; Responses API
  `event: <type>` frames; Gemini parts frames). Follow the JSON shapes
  already used in `doppel.rs` / `sse_restore.rs` inline tests.
- `async fn collect_all(restore: SseRestoreStream) -> String`: drop the
  sender, drain the stream to EOF inside a
  `tokio::time::timeout(Duration::from_secs(5), …)`, concatenate all output
  bytes lossily into one `String`.
- `fn pos(haystack: &str, needle: &str) -> usize`: `haystack.find(needle)`
  with a panic message naming the needle. Ordering assertions compare byte
  offsets in the concatenated output. This is sufficient: `output_queue` is
  FIFO, so pre-fix behavior (terminal pushed before the held content flushes
  at EOF) yields `pos(secret) > pos(terminal)` and the test fails.

**Critical sizing rule:** for every held-content scenario, the accumulated
text fed before the terminal must total EXACTLY the fake string (split across
2–3 delta frames, e.g. `fake[..10]`, `fake[10..25]`, `fake[25..]`). Total
length == `max_fake_len`, so no safe-prefix flush fires early and the
terminal-triggered complete flush emits the entire restored secret in one
synthetic frame — making `pos(secret)` well-defined. Do not append extra
bytes around the fake (a safe-prefix flush could split it across frames and
break the contiguous-substring search).

### Spec invariant tests (one per VC)

1. `vc_sse_14_anthropic_block_stop_ordering` — Provider::Anthropic. Feed the
   fake split across `input_json_delta` frames for index 0, then
   `content_block_stop` index 0, then `message_stop`. Assert
   `pos(secret) < pos("content_block_stop")` and the fake is absent.

2. `vc_sse_15_anthropic_message_stop_ordering` — fake split across
   `text_delta` frames index 0, then `message_stop` directly (no block stop).
   Assert `pos(secret) < pos("message_stop")`. Add the same shape for
   `message_delta` (either a second case in this test or a sibling test):
   deltas → `message_delta` → assert `pos(secret) < pos("message_delta")`.

3. `vc_sse_16_anthropic_block_stop_isolation` — two active buffers:
   index 0 `text_delta` frames carrying the fake; index 1 `input_json_delta`
   frames carrying a distinct marker (e.g. `"VC16_BLOCK1_MARKER"`, shorter
   than `max_fake_len` so it stays fully held). Send `content_block_stop`
   index 0 only, then EOF. Assert:
   - `pos(secret) < pos("content_block_stop")` (block 0 flushed before stop);
   - `pos("VC16_BLOCK1_MARKER") > pos("content_block_stop")` (block 1 NOT
     flushed by block 0's stop; it drains only at EOF);
   - marker is present in the output (EOF flush emitted it).

4. `vc_sse_17_openai_finish_reason_ordering` — Provider::OpenAi. Fake split
   across `tool_calls[0].function.arguments` delta frames, then
   `{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}`, then
   `data: [DONE]\n\n`. Assert `pos(secret) < pos("finish_reason")` and
   `pos(secret) < pos("[DONE]")`.

5. `vc_sse_18_responses_api_done_ordering` — Provider::OpenAi. Fake split
   across `event: response.output_text.delta` frames (`"delta"` field), then
   `event: response.content_part.done`, `event: response.output_item.done`,
   `event: response.completed` frames (path-A shapes: no extractable text).
   Assert `pos(secret) < pos("response.content_part.done")`, and that
   `response.output_item.done` and `response.completed` also appear after
   `pos(secret)`.

6. `vc_sse_19_gemini_finish_reason_colocated` — Provider::Gemini. A single
   frame whose `candidates[0]` has BOTH `content.parts[0].text` containing
   the fake AND `"finishReason":"STOP"`. Assert: secret present in output,
   fake absent, and the frame was processed as content (path B) — i.e. the
   restored text appears in a synthetic frame and the stream completes
   without error. No ordering inversion is possible by construction; the
   test guards that co-located frames are NOT misrouted to the path-A
   terminal handling (which would drop their content).

7. `vc_sse_20_gemini_empty_terminal_ordering` — Provider::Gemini. Fake split
   across text-part frames, then a frame with
   `{"candidates":[{"finishReason":"STOP"}]}` and no parts (extract_fields
   returns empty). Assert `pos(secret) < pos("finishReason")`.

Register the module: add `mod sse_terminal_ordering;` to `tests/spec/mod.rs`
(keep alphabetical order with the existing entries).

### Integration regression test (wire-level)

Add `sse_terminal_frames_ordered_after_restored_content` to
`tests/integration/doppel.rs`, mirroring
`restore_anthropic_input_json_delta`: register the secret, build a
`MockUpstream::builder().sse(...)` Anthropic stream where the fake spans
multiple `input_json_delta` chunks for block 0 followed by
`content_block_stop(0)`, `message_delta`, `message_stop`; request through the
harness with the secret in the request body (the real-world trigger: doppel
detects it, so `max_fake_len > 0`). On the collected client bytes assert:

- the full secret is present and the fake absent (existing helpers);
- `pos(secret) < pos("content_block_stop")` and
  `pos(secret) < pos("message_stop")`;
- concatenating every `partial_json` fragment from the client's
  `content_block_delta` frames for index 0 parses with
  `serde_json::from_str::<serde_json::Value>` — the downstream-truncation
  symptom from the bug report, now impossible.

## Acceptance Criteria

- [ ] `cargo nextest run --test spec` exits 0 and lists all 7 new
      `vc_sse_*` tests as PASS
- [ ] `cargo nextest run --test integration` exits 0 and lists
      `sse_terminal_frames_ordered_after_restored_content` as PASS
- [ ] `cargo nextest run` exits 0 (full suite)
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] Regression linkage verified: with step 02's `process_one_frame` wiring
      temporarily reverted (e.g. `git stash` of the branch edits or
      `git revert --no-commit <step-02-sha>` in a scratch worktree state),
      `cargo nextest run --test spec` FAILS on at least
      `vc_sse_14_anthropic_block_stop_ordering` and
      `vc_sse_17_openai_finish_reason_ordering`; restore the fix afterwards

## Reviewer Instructions

- Map each test to its VC: every one of VC-SSE-14..20 has exactly one
  covering test asserting the MUST clause, including VC-SSE-16's negative
  assertion (block 1 NOT flushed early).
- Check the sizing rule: held text before a terminal totals exactly the fake;
  no early safe-prefix flush can split the secret across synthetic frames.
- Check no real credentials: the key is the synthetic structural-match string
  already used in `sse_restore_streaming.rs`, with the same "NOT a real
  credential" comment.
- Check all stream polls are wrapped in `tokio::time::timeout` — a regression
  must fail the assertion, not hang CI.
- These are external behavioral contracts (`tests/spec/`, `tests/integration/`)
  — confirm no existing external test was weakened or modified to pass.
- Confirm the regression-linkage acceptance item was actually performed (the
  step report must state the observed failures against the reverted fix).

## Rollback

`git revert` the step commit (removes `tests/spec/sse_terminal_ordering.rs`,
the `mod` registration line, and the one integration test). No production
code is touched by this step, so rollback has zero runtime impact — but
reverting removes the behavioral contract for VC-SSE-14..20 and should only
accompany a deliberate spec change.
