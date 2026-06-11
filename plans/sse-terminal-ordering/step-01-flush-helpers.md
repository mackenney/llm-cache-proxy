# Step 01: Flush Helpers (refactor, no behavior change)

## Context

### Overall Objective

Fix the terminal-event ordering bug in `SseRestoreStream`: terminal SSE frames
are forwarded on path A of `process_one_frame` before held accumulator content
is flushed, violating SPEC.md §Terminal Event Ordering (VC-SSE-14..20).

### Phase Context

Wave 1 of 3. This step only prepares the flush machinery. Step 02 wires
terminal detection into `process_one_frame` using the helper introduced here.

### This Step

Introduce a predicate-scoped complete-flush helper in
`crates/lcp-server/src/ext/sse_restore.rs` and route the existing
`flush_all_accumulators` (line ~1006) through it, so step 02 can flush either
ALL buffers (stream scope) or only `AnthropicDelta { index == N }` buffers
(block scope) with the same code path. Also remove an existing exact-duplicate
`flush_safe_prefix` call in the path-B loop of `process_one_frame`.

**No observable behavior change in this step.**

## Prerequisites

- None (first step). Working tree clean, `cargo nextest run` green at baseline.

## Files to Read Before Starting

- `crates/lcp-server/src/ext/sse_restore.rs` — focus on:
  - `process_one_frame` (~line 862), especially the flush loop at ~941–959
  - `flush_safe_prefix` (~line 968)
  - `flush_all_accumulators` (~line 1006) and its EOF call site (~line 767)
  - `FieldKey` enum (~line 26)
- `crates/lcp-server/SPEC.md` §Terminal Event Ordering (lines ~436–518) — for
  the "complete flush" definition (hold window of zero).

## Implementation

1. Add a predicate-based helper next to `flush_all_accumulators`:

   ```rust
   /// Completely flushes (hold window = 0) every accumulation buffer whose
   /// key satisfies `pred`. Used for terminal-event flushes (SPEC.md
   /// §Terminal Event Ordering) and, via flush_all_accumulators, at EOF.
   fn flush_accumulators_where(
       accumulators: &mut BTreeMap<FieldKey, String>,
       pred: impl Fn(&FieldKey) -> bool,
       entries: &[Entry],
       session_key: &SessionKey,
       output_queue: &mut VecDeque<Bytes>,
   ) -> io::Result<()> {
       for (key, accum) in accumulators.iter_mut() {
           if accum.is_empty() || !pred(key) {
               continue;
           }
           flush_safe_prefix(key, accum, 0, entries, session_key, output_queue)?;
       }
       Ok(())
   }
   ```

2. Reimplement `flush_all_accumulators` as a thin wrapper:

   ```rust
   fn flush_all_accumulators(...) -> io::Result<()> {
       flush_accumulators_where(accumulators, |_| true, entries, session_key, output_queue)
   }
   ```

   Keep its existing signature and doc comment (still called at stream EOF,
   ~line 767). Routing the EOF path through the new helper means the helper is
   live code — no `dead_code` warnings, no `#[allow]`.

3. In `process_one_frame`, the path-B flush loop (~lines 941–959) calls
   `flush_safe_prefix` twice with identical arguments. The second call is a
   guaranteed no-op (after the first call `accum.len() <= max_fake_len`, which
   early-returns). Delete the duplicate so the loop calls `flush_safe_prefix`
   exactly once per key. This is dead work removal, not a behavior change.

4. Add inline unit tests to the existing `#[cfg(test)]` module in
   `sse_restore.rs`:
   - `flush_where_only_matching_keys`: two accumulators
     (`AnthropicDelta { delta_type: "text_delta", index: 0 }` and
     `AnthropicDelta { delta_type: "input_json_delta", index: 1 }`) with
     non-empty plain text, empty `entries`; flush with predicate
     `index == 0`; assert index-0 buffer is empty and its text appeared in
     `output_queue`, index-1 buffer untouched and absent from `output_queue`.
   - `flush_where_true_flushes_all`: same setup, predicate `|_| true`; assert
     both buffers drained.

Do NOT touch `process_one_frame`'s path A, `extract_fields`, or any detection
logic in this step.

## Acceptance Criteria

- [ ] `cargo nextest run` exits 0 (all tiers, no regressions)
- [ ] `cargo nextest run --test spec` exits 0
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo fmt --check` exits 0
- [ ] `grep -c "flush_safe_prefix(" crates/lcp-server/src/ext/sse_restore.rs` shows the duplicate call removed (one call site in the path-B loop, one in `flush_accumulators_where`, plus the definition)

## Reviewer Instructions

- Confirm `flush_accumulators_where` passes `max_fake_len = 0` to
  `flush_safe_prefix` (complete flush per SPEC.md §Terminal Event Ordering).
- Confirm `flush_all_accumulators` semantics are unchanged: skip-empty
  behavior preserved, same signature, EOF call site (~line 767) untouched.
- Confirm the deleted code in `process_one_frame` was an exact duplicate call
  with identical arguments (`git diff` should show a pure deletion there).
- Confirm zero behavior change: no path-A edits, no detection logic, no new
  public API.
- Comment style: no section separators; comments explain WHY.

## Rollback

`git revert` the step commit. The change is a self-contained refactor inside
`sse_restore.rs` with no callers outside the file; reverting restores the
previous `flush_all_accumulators` body and the duplicated (harmless) flush
call.
