# Progress

## Status
Complete

## Tasks
- [x] Read context.md, plan.md, and SPEC files
- [x] Analyze hash.rs, provider.rs, proxy.rs, harness.rs
- [x] Produce implementation plan for core architecture

## Deliverable
- `/home/ignacio/pr/llm-cache-proxy/artifacts/plan-norm-core.md`

## Summary
Implementation plan covers:
1. `Provider::fields_to_strip()` method (provider.rs)
2. `cache_key(provider, method, path, body)` signature change (hash.rs)
3. `normalize_body` internals — two-pass strip: universal + provider-specific
4. Call site update in proxy.rs:79
5. Inline unit test updates in hash.rs

Key decisions:
- Provider method follows `extract_model_from_path` pattern
- `stream` stays hardcoded (universal); provider method returns attribution/routing fields only
- Breaking API change (single call site makes this safe)
- Existing recursive `strip_fields` handles nested `metadata` for Anthropic

## Notes
- Harness does NOT call cache_key directly — no harness changes needed
- Another planner covers tests/migration — not included here

---

# Tests & Harness Plan (This Planner)

## Status
Planning complete

## Deliverable
- `/home/ignacio/pr/llm-cache-proxy/artifacts/plan-norm-tests.md`

## Tasks
- [ ] Step 1: Update 6 hash.rs unit tests for new `cache_key(Provider, ...)` signature
- [ ] Step 2: Create `tests/spec/normalization.rs` with 9 new spec invariant tests
- [ ] Step 3: Register normalization module in `tests/spec/mod.rs`
- [ ] Step 4: Verify harness (no changes expected)

## Files to Change
| File | Change |
|---|---|
| `crates/lcp-core/src/hash.rs:77-131` | Add Provider arg to all 6 unit test calls |
| `tests/spec/normalization.rs` | New file: 9 tests for per-provider strip lists |
| `tests/spec/mod.rs` | Add `mod normalization;` |

## Key Decisions
- Unit tests use `Provider::Anthropic` arbitrarily (mechanics are provider-agnostic)
- Spec tests follow `model_extraction.rs` pattern (harness → HTTP → proxy → cache)
- Harness unchanged (provider resolved from URL prefix in proxy layer)

## Spec Test Coverage
| Test | What it verifies |
|---|---|
| `test_anthropic_metadata_stripped` | `metadata` field stripped for Anthropic |
| `test_anthropic_thinking_NOT_stripped` | `thinking` field NOT stripped |
| `test_openai_user_stripped` | `user` field stripped for OpenAI |
| `test_openrouter_user_stripped` | `user` field stripped for OpenRouter |
| `test_openrouter_provider_route_stripped` | `provider`, `route` stripped |
| `test_openrouter_transforms_NOT_stripped` | `transforms` NOT stripped |
| `test_openrouter_reasoning_NOT_stripped` | `reasoning` NOT stripped |
| `test_cross_provider_same_body_different_keys` | Provider included in key |
| `test_gemini_no_body_fields_stripped` | Only `stream` stripped for Gemini |

## Coordination Note
Unit test updates (Step 1) should land in same commit as the signature change from the core plan to avoid broken intermediate state.
