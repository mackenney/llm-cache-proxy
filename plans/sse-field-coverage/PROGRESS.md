# PROGRESS.md

## Status
In Progress

## Objective
Extend the SSE restore pipeline to handle all provider-specific content fields (VC-SSE-1..13), replacing the single-field/single-buffer architecture with a keyed multi-buffer model that accumulates and restores each content field independently.

## Open Decisions
None — all architectural and test-strategy decisions are resolved.

## Wave Map
| Wave | Steps | Can Parallelize | Depends On |
|---|---|---|---|
| 0 — Refactor internals | step-01 | No | — |
| 1 — Anthropic new fields | step-02 | No | Wave 0 |
| 2 — OpenAI/OpenRouter new fields | step-03 | No | Wave 0 |
| 3 — Responses API | step-04 | No | Wave 0 |
| 4 — Gemini new fields | step-05 | No | Wave 0 |
| 5 — E2E tests | step-06 | No | Waves 1–4 |
| 6 — Cross-field isolation + regression | step-07 | No | Waves 1–4 |

Note: Waves 1–4 can execute in parallel after Wave 0 completes. Wave 5 and Wave 6 both depend on Waves 1–4 but are independent of each other and can run in parallel.

## Dependency Table
| Step | File(s) | Depends On | Depended By |
|---|---|---|---|
| step-01 | `crates/lcp-server/src/ext/sse_restore.rs` | — | step-02, step-03, step-04, step-05, step-06, step-07 |
| step-02 | `crates/lcp-server/src/ext/sse_restore.rs`, `tests/integration/doppel.rs` | step-01 | step-06, step-07 |
| step-03 | `crates/lcp-server/src/ext/sse_restore.rs`, `tests/integration/doppel.rs` | step-01 | step-06, step-07 |
| step-04 | `crates/lcp-server/src/ext/sse_restore.rs`, `tests/integration/doppel.rs` | step-01 | step-06, step-07 |
| step-05 | `crates/lcp-server/src/ext/sse_restore.rs`, `tests/integration/doppel.rs` | step-01 | step-06, step-07 |
| step-06 | `tests/e2e/` | step-02, step-03, step-04, step-05 | — |
| step-07 | `tests/integration/doppel.rs` | step-02, step-03, step-04, step-05 | — |

## Orchestrator Protocol
1. Read this file to identify current wave
2. Dispatch all steps in current wave in parallel
3. After each step: run acceptance criteria commands; mark complete only on pass
4. Advance to next wave only when all steps in current wave complete
5. Blockers: stop and report to user

## Subagent Contract
- Workers: Read step file fully before acting. Implement only what the step specifies.
- Workers: Commit with message "step-NN: <name>"
- Workers: Report "Step NN complete ✅ (commit <hash>)" or "Step NN FAILED: <reason>"
- Reviewers: Run acceptance criteria verbatim. Pass or fail with specifics.

## Steps
- [ ] [step-01-refactor-api](./step-01-refactor-api.md) — Wave 0: Replace single-field extraction with `FieldKey`/`ExtractedField` multi-buffer architecture; all existing tests must pass
- [ ] [step-02-anthropic-fields](./step-02-anthropic-fields.md) — Wave 1: Add `thinking_delta` (VC-SSE-1), `input_json_delta` (VC-SSE-2), `signature_delta` passthrough (VC-SSE-3) with unit + integration tests
- [ ] [step-03-openai-fields](./step-03-openai-fields.md) — Wave 2: Add `tool_calls[N].function.arguments` (VC-SSE-4), `reasoning_content` (VC-SSE-5), `function_call.arguments` (VC-SSE-6) with unit + integration tests
- [ ] [step-04-responses-api](./step-04-responses-api.md) — Wave 3: Add OpenAI Responses API support (VC-SSE-7, VC-SSE-8) with unit + integration tests
- [ ] [step-05-gemini-fields](./step-05-gemini-fields.md) — Wave 4: Add multi-part text (VC-SSE-9), `codeExecutionResult.output` (VC-SSE-10), `functionCall.args` (VC-SSE-11), metadata passthrough (VC-SSE-12) with unit + integration tests
- [ ] [step-06-e2e-tests](./step-06-e2e-tests.md) — Wave 5: E2E tests per provider, gated by env vars
- [ ] [step-07-cross-field-regression](./step-07-cross-field-regression.md) — Wave 6: Cross-field isolation (VC-SSE-13) integration tests + full regression sweep

## Owner Attention Required

- **API keys for E2E (step-06):** Ensure `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GEMINI_API_KEY` are set in the execution environment. ✅ Confirmed available.

All other decisions resolved:
- `gpt-5.5-pro` for Responses API E2E — confirmed ✅
- `includeThoughts: true` set in Gemini E2E request — confirmed ✅
- JSON traversal not needed — byte-stream restore is sufficient — confirmed ✅
- `delta.refusal` upgraded to MUST — confirmed ✅