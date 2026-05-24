# Progress

## Status
Complete

---

## Implementation Planner (artifacts/plan-gemini-impl.md)

### Tasks
- [x] Analyze Provider enum in lcp-core/src/provider.rs
- [x] Analyze extract_model() in lcp-server/src/proxy.rs
- [x] Review SPEC constraints (cache key invariant, dependency direction)
- [x] Write implementation plan to artifacts/plan-gemini-impl.md

### Key Decisions
1. Add `Provider::extract_model_from_path()` method in lcp-core
2. Use simple string parsing (find + slice) over regex
3. Fallback chain: path extraction → body extraction
4. Return bare model name (consistent with other providers)

### Notes
- Cache keys already include full path with model — no change needed
- Dependency direction preserved (server → core)
- Other providers unaffected (their path extraction returns None)

---

## Spec + Tests Planner (artifacts/plan-gemini-spec-tests.md)

### Tasks
- [x] Read SPEC.md lines 271-274 (by_model documentation)
- [x] Read crates/lcp-server/SPEC.md (proxy behavior, model extraction)
- [x] Read crates/lcp-core/SPEC.md (CacheStats, by_model)
- [x] Review existing spec tests (cache_hit.rs, cache_miss.rs patterns)
- [x] Review test infrastructure (TestHarness, MockUpstream)
- [x] Write spec+tests plan to artifacts/plan-gemini-spec-tests.md

### Spec Changes Planned
1. **crates/lcp-server/SPEC.md** — New "Model Extraction" subsection with per-provider table
2. **crates/lcp-core/SPEC.md** — Update CacheStats.by_model description (remove body-only assumption)
3. **SPEC.md** — Update by_model documentation to explain Gemini path extraction

### Tests Planned
1. `tests/spec/model_extraction.rs` — New test file
2. `test_gemini_model_extracted_from_path` — Core GEMINI-1 fix validation
3. `test_gemini_model_appears_in_stats_by_model` — Stats integration
4. `test_gemini_stream_generate_content_model_extracted` — Alternative verb
5. `test_anthropic_model_extracted_from_body` — Regression guard
6. `test_openai_model_extracted_from_body` — Regression guard

### Notes
- All tests use existing TestHarness + MockUpstream infrastructure
- No new dependencies required
- Tests should fail (red) before implementation merges, pass after (green)
