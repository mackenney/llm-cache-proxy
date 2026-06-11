# Step 02: Terminal Detection and Flush Wiring (behavior change)

## Context

### Overall Objective

Fix the terminal-event ordering bug in `SseRestoreStream`: terminal SSE frames
are forwarded on path A of `process_one_frame` before held accumulator content
is flushed, violating SPEC.md §Terminal Event Ordering (VC-SSE-14..20).

### Phase Context

Wave 2 of 3. Step 01 provided `flush_accumulators_where` (predicate-scoped
complete flush). This step is the actual fix. Step 03 adds the per-VC spec
invariant tests.

### This Step

Add terminal-event classification and wire complete flushes into the two
early-return paths of `process_one_frame` in
`crates/lcp-server/src/ext/sse_restore.rs`:

- path A (`extract_fields` returned empty, ~line 885): classify the frame;
  block-scope or stream-scope flush before forwarding.
- non-JSON branch (~line 878): `data: [DONE]` triggers a stream-scope flush
  before forwarding.

## Prerequisites

- Step 01 merged: `flush_accumulators_where` exists and
  `cargo nextest run` is green.

## Files to Read Before Starting

- `crates/lcp-server/SPEC.md` §Terminal Event Ordering (lines ~436–518):
  per-provider terminal tables, scope definitions, and the explicit
  non-terminal list. This is the contract — implement exactly it.
- `crates/lcp-server/src/ext/sse_restore.rs`:
  - `process_one_frame` (~line 862): both early-return branches and the
    `SseCtx` fields (`entries`, `session_key`, `provider`, `max_fake_len`)
  - `FieldKey` enum (~line 26)
  - `flush_accumulators_where` / `flush_all_accumulators` (from step 01)
  - `extract_fields` (~line 94) — to understand which frames land on path A
    per provider (e.g., Responses API `response.output_text.done` carries
    text and goes path B; it is NOT in the terminal list)
- `crates/lcp-core/src/provider.rs`: `Provider` enum
  (`Anthropic | OpenAi | OpenRouter | Gemini`).

## Implementation

1. Add a classification type and function near `process_one_frame`:

   ```rust
   /// Scope of a terminal SSE event per SPEC.md §Terminal Event Ordering.
   enum TerminalScope {
       /// Terminates one Anthropic content block; flush only
       /// `AnthropicDelta { index == N }` buffers (VC-SSE-16 isolation).
       Block(u64),
       /// Terminates the whole response; flush all buffers.
       Stream,
   }

   fn classify_terminal(
       json: &serde_json::Value,
       event_type: Option<&str>,
       provider: Provider,
   ) -> Option<TerminalScope>
   ```

   Classification is provider-gated (match on `provider`):

   - **`Provider::Anthropic`** — on `json["type"].as_str()`:
     - `"content_block_stop"` → `Block(json["index"].as_u64())`. If `index`
       is missing or non-u64 (malformed frame), fall back to `Stream`:
       over-flushing preserves ordering; under-flushing reintroduces the bug.
     - `"message_delta"` | `"message_stop"` → `Stream`.
     - anything else (`"ping"`, `"message_start"`, `"content_block_start"`,
       unknown) → `None`.
   - **`Provider::OpenAi` | `Provider::OpenRouter`** —
     - Responses API: if `event_type` is one of
       `"response.content_part.done"`, `"response.output_item.done"`,
       `"response.completed"`, `"response.failed"`,
       `"response.cancelled"`, `"response.incomplete"` → `Stream`.
       (`response.created`, `response.in_progress`,
       `response.output_item.added`, `response.content_part.added` → `None`.)
     - Chat Completions: if `json.pointer("/choices/0/finish_reason")`
       returns `Some(v)` with `!v.is_null()` → `Stream`.
     - otherwise `None`.
   - **`Provider::Gemini`** — defensive only (live streams co-locate
     `finishReason` with content, which lands on path B per VC-SSE-19):
     `json.pointer("/candidates/0/finishReason").and_then(|v| v.as_str()).is_some()`
     → `Stream`; otherwise `None`. This is only ever evaluated on path A,
     i.e., when `extract_fields` already returned empty (VC-SSE-20).

2. Wire into `process_one_frame` path A (the `extracted.is_empty()` branch,
   ~lines 885–889). Before pushing the frame to `output_queue`:

   ```rust
   if extracted.is_empty() {
       match classify_terminal(&json, event_type, ctx.provider) {
           Some(TerminalScope::Block(n)) => flush_accumulators_where(
               accumulators,
               |k| matches!(k, FieldKey::AnthropicDelta { index, .. } if *index == n),
               ctx.entries,
               ctx.session_key,
               output_queue,
           )?,
           Some(TerminalScope::Stream) => flush_all_accumulators(
               accumulators, ctx.entries, ctx.session_key, output_queue,
           )?,
           None => {}
       }
       output_queue.push_back(Bytes::from(frame.as_bytes().to_vec()));
       return Ok(());
   }
   ```

3. Wire into the non-JSON branch (~lines 878–882). `[DONE]` is non-JSON, so
   it never reaches `classify_terminal`; handle it here:

   ```rust
   let Ok(json) = serde_json::from_str::<serde_json::Value>(data_str) else {
       // "[DONE]" is the Chat Completions stream terminator: complete-flush
       // all buffers before forwarding (SPEC VC-SSE-17). Other non-JSON data
       // passes through without a flush.
       if data_str.trim() == "[DONE]" {
           flush_all_accumulators(accumulators, ctx.entries, ctx.session_key, output_queue)?;
       }
       output_queue.push_back(Bytes::from(frame.as_bytes().to_vec()));
       return Ok(());
   };
   ```

4. Leave untouched:
   - the no-`data:`-line branch (~lines 872–876, SSE comments/keep-alives) —
     no flush;
   - path B (non-empty `extracted`) — accumulation + safe-prefix flush
     unchanged;
   - the non-SSE (plain JSON) response path elsewhere in the file;
   - EOF flush (~line 767).

5. Inline unit tests in the existing `#[cfg(test)]` module for
   `classify_terminal` (table-driven where convenient):
   - Anthropic: `content_block_stop` index 0 / index 3 → `Block(0)` /
     `Block(3)`; `content_block_stop` without `index` → `Stream`;
     `message_delta`, `message_stop` → `Stream`; `ping`, `message_start`,
     `content_block_start` → `None`.
   - OpenAi: finish-reason frame
     `{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}` → `Stream`;
     `{"choices":[{"delta":{"content":"x"},"finish_reason":null}]}` → `None`;
     each of the six `response.*` terminal event types → `Stream`;
     `response.created` / `response.output_item.added` /
     `response.content_part.added` / `response.in_progress` → `None`.
   - OpenRouter: finish-reason frame → `Stream` (same shape as OpenAi).
   - Gemini: `{"candidates":[{"finishReason":"STOP"}]}` → `Stream`; frame
     without `finishReason` → `None`.
   - Cross-provider guard: an Anthropic-shaped `message_stop` JSON under
     `Provider::Gemini` → `None` (classification is provider-gated).
   - One `process_one_frame`-level test: accumulator for
     `AnthropicDelta { index: 0 }` holding text, feed a `content_block_stop`
     index-0 frame, assert `output_queue` contains the synthetic content
     frame BEFORE the stop frame.

Do not log, persist, or branch on accumulator *contents* anywhere in the new
code — `SensitiveState` and held text MUST NOT be inspected beyond the
existing flush/restore machinery.

## Acceptance Criteria

- [ ] `cargo nextest run` exits 0 (all existing tiers green — proves cache-hit
      and non-SSE paths unaffected)
- [ ] `cargo nextest run --test spec` exits 0
- [ ] `cargo nextest run --test integration` exits 0
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] New `classify_terminal` unit tests pass:
      `cargo nextest run -p lcp-server classify_terminal` exits 0 with >0 tests run

## Reviewer Instructions

- Check the classification table 1:1 against SPEC.md §Terminal Event Ordering
  (lines ~460–518): every listed terminal triggers the right scope; every
  listed non-terminal (`ping`, `message_start`, `content_block_start`,
  `response.created`, `response.in_progress`, `response.output_item.added`,
  `response.content_part.added`) returns `None`.
- Verify block-scope isolation: the predicate matches ONLY
  `FieldKey::AnthropicDelta` with the exact index; all other variants
  (including `AnthropicDelta` with a different index) are excluded
  (VC-SSE-16).
- Verify flushes happen BEFORE the `output_queue.push_back` of the terminal
  frame in both wired branches — `output_queue` is FIFO, so push order is
  emission order.
- Verify the no-data-line branch and path B are byte-identical to before
  (besides the step-01 dedup already merged).
- Verify `[DONE]` matching is on `data_str.trim()`, not a substring search.
- Confirm nothing inspects, logs, or persists accumulator contents or
  `SensitiveState`.

## Rollback

`git revert` the step commit. All edits are confined to `sse_restore.rs`
(`classify_terminal`, `TerminalScope`, two branch edits in
`process_one_frame`, unit tests); reverting restores the pre-fix passthrough
behavior without affecting step 01's helpers.
