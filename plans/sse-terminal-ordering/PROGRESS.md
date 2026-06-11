# PROGRESS.md

## Status

Queued. No step started.

## Objective

Fix the terminal-event ordering bug in `SseRestoreStream`
(`crates/lcp-server/src/ext/sse_restore.rs`): `process_one_frame` forwards
non-content SSE frames (path A) without flushing per-`FieldKey` accumulation
buffers first. Terminal events (`content_block_stop`, `message_stop`,
`[DONE]`, `response.content_part.done`, `response.output_item.done`, …) reach
the client before the held content they terminate, truncating tool-call
arguments downstream whenever doppel holds bytes (`max_fake_len > 0`).

Contract: SPEC.md (`crates/lcp-server/SPEC.md`) §Terminal Event Ordering and
VC-SSE-14 through VC-SSE-20. Two flush strategies:

- **Block-scope** (Anthropic `content_block_stop(index=N)`): complete-flush
  only `AnthropicDelta { index == N }` buffers (VC-SSE-16 isolation).
- **Stream-scope** (everything else): complete-flush ALL buffers.

"Complete flush" = hold window of zero. Non-SSE (plain JSON) path, cache-hit
behavior, and `SensitiveState` handling are untouched.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|---|---|---|---|
| 1 | step-01 | No | — |
| 2 | step-02 | No | Wave 1 |
| 3 | step-03 | No | Wave 2 |

## Steps

- [ ] [step-01-flush-helpers](./step-01-flush-helpers.md) — predicate-based complete-flush helper; refactor `flush_all_accumulators` through it; remove duplicated `flush_safe_prefix` call; no behavior change
- [ ] [step-02-terminal-detection](./step-02-terminal-detection.md) — `classify_terminal` + wiring into `process_one_frame` path A and the `[DONE]` branch; behavior change
- [ ] [step-03-spec-tests](./step-03-spec-tests.md) — spec invariant tests for VC-SSE-14..20 in `tests/spec/sse_terminal_ordering.rs` + one wire-level integration regression test
