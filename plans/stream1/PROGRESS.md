# PROGRESS.md

## Status
Complete

## Objective
Replace full-body buffering in proxy.rs with true streaming using Axum's `Body::from_stream()`. Cache misses use spawn+channel to tee upstream chunks to both client and cache accumulator. Cache hits stream directly from stored chunks.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 0    | step-01, step-02 | Yes | — |
| 1    | step-03 | No | Wave 0 |
| 2    | step-04 | No | Wave 1 |

## Dependency Table

| Step | Depends On | Description |
|------|------------|-------------|
| step-01 | — | Add ReceiverStream adapter and imports |
| step-02 | — | Refactor cache hit path to stream |
| step-03 | step-01 | Refactor cache miss path with spawn+channel |
| step-04 | step-03 | Integration validation and final checks |

## Orchestrator Protocol
1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: dispatch reviewer agent
4. Mark step complete only after reviewer passes
5. Advance to next wave only when all steps in current wave are complete
6. Blockers: stop and report to user with full context

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit changes with message "step-NN: <name>"
- Workers: Report back: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim. Pass or fail with specifics.

## Key Invariants (from correctness analysis)
1. **Complete-or-nothing caching:** A cached exchange represents a full successful response only.
2. **Partial failures not cached:** Upstream error mid-stream → no cache write.
3. **Client disconnect doesn't corrupt cache:** Either cache complete response or nothing.
4. **Chunk boundaries preserved:** Same `Vec<ResponseChunk>` structure, yielded in order.
5. **Backpressure respected:** Bounded channel prevents runaway memory.

## Steps
- [x] [step-01-add-receiver-stream](./step-01-add-receiver-stream.md) — Add ReceiverStream adapter and imports (25e5901)
- [x] [step-02-cache-hit-streaming](./step-02-cache-hit-streaming.md) — Refactor serve_cached() to use stream::iter() (25e5901)
- [x] [step-03-cache-miss-streaming](./step-03-cache-miss-streaming.md) — Refactor cache miss with spawn+channel pattern (25e5901)
- [x] [step-04-validation](./step-04-validation.md) — Integration validation and cleanup (step-04 commit)

## Open Decisions
None — both analysis plans agree on the architecture.

## Verified Facts
- `crates/lcp-server/src/proxy.rs:110-129`: chunks buffered into `full_body: Vec<u8>` before response
- `crates/lcp-server/src/proxy.rs:175`: `Body::from(full_body)` — full buffer sent at once
- `crates/lcp-server/src/proxy.rs:179-193`: `serve_cached()` also concatenates to Vec
- `Cargo.toml`: axum 0.8, reqwest 0.12 stream feature, futures-util 0.3, bytes 1, tokio full
- `futures_util::StreamExt` already imported at proxy.rs line 9
- `tokio::sync::mpsc` available via tokio full features
- `Body::from_stream()` available in axum 0.8
- No new dependencies needed
