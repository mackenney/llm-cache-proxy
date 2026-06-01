# PROGRESS.md

## Status

In Progress

## Objective

Implement SSE-aware unscrubbing in `ScrubExt::on_response_stream` so that fake keys
split across SSE `data:` events are detected at the semantic text level and replaced
with the original decrypted values before the response reaches the client or cache.

## Wave Map

| Wave | Steps | Can Parallelize | Depends On |
|------|-------|-----------------|------------|
| 1 | 01 | — | nothing |
| 2 | 02 | — | step-01 |
| 3 | 03, 05 | yes | step-02 |
| 4 | 04 | — | step-03 |

## Dependency Table

| Step | File(s) | Depends On | Depended By |
|------|---------|------------|-------------|
| 01 | `crates/lcp-server/src/ext/sse_unscrub.rs`, `ext/mod.rs` | — | 02, 03 |
| 02 | `crates/lcp-server/src/ext/sse_unscrub.rs` | 01 | 03, 05 |
| 03 | `crates/lcp-server/src/ext/scrub.rs` | 02 | 04 |
| 04 | `tests/integration/scrub.rs` | 03 | — |
| 05 | `crates/lcp-server/SPEC.md`, `MASTER_PROGRESS.md` | 02 | — |

## Orchestrator Protocol

1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: dispatch reviewer agent
4. Mark step complete only after reviewer passes
5. Advance to next wave only when all steps in wave are complete

## Subagent Contract

- Workers: Read step file fully before acting. Commit with message "step-NN: <name>"
- Workers: Report back: "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria commands verbatim. Pass or fail with specifics.

## Steps

- [ ] [step-01-sse-helpers](./step-01-sse-helpers.md) — New `sse_unscrub.rs` module: SSE detection and provider text-field helpers with unit tests
- [ ] [step-02-sse-unscrub-stream](./step-02-sse-unscrub-stream.md) — `SseUnscrubStream` type: state machine buffering SSE events, unscrubbing accumulated text, redistributing
- [ ] [step-03-wire-scrub-ext](./step-03-wire-scrub-ext.md) — Wire `SseUnscrubStream` into `ScrubExt::on_response_stream`; Anthropic SSE integration test passes
- [ ] [step-04-provider-tests](./step-04-provider-tests.md) — Integration tests for OpenAI and Gemini SSE formats
- [ ] [step-05-spec-update](./step-05-spec-update.md) — Replace `TODO` preamble in `SPEC.md §SSE-Aware Unscrubbing` with normative text; update `MASTER_PROGRESS.md`
