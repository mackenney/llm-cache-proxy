# Test Coverage Audit: lcp

**Date**: 2026-05-24  
**Commit**: Current HEAD  
**SPEC Baseline**: SPEC.md (main), crates/lcp-core/SPEC.md, crates/lcp-server/SPEC.md

---

## 1. What IS Tested

### Unit Tests (inline, `#[cfg(test)]` modules)

| File | Test | SPEC Coverage |
|---|---|---|
| `crates/lcp-core/src/hash.rs:77-130` | `identical_bodies_same_key()` | Cache Key: deterministic hashing |
| `crates/lcp-core/src/hash.rs:86-93` | `stream_field_stripped()` | Cache Key rule 2: strip `stream` field |
| `crates/lcp-core/src/hash.rs:96-103` | `key_order_independent()` | Cache Key rule 3: sort keys recursively |
| `crates/lcp-core/src/hash.rs:106-113` | `different_bodies_different_keys()` | Cache Key: body discrimination |
| `crates/lcp-core/src/hash.rs:116-122` | `different_paths_different_keys()` | Cache Key: path discrimination |
| `crates/lcp-core/src/hash.rs:125-130` | `invalid_json_hashes_raw()` | Cache Key rule 1: fallback to raw bytes |
| `crates/lcp-core/src/cache.rs:433-436` | `miss_on_empty_cache()` | Cache read on empty |
| `crates/lcp-core/src/cache.rs:439-451` | `put_then_get_roundtrip()` | Cache write and read |
| `crates/lcp-core/src/cache.rs:454-464` | `hit_count_increments()` | Cache: `hit_count` increments on hit |
| `crates/lcp-core/src/cache.rs:467-478` | `stats_track_hits_and_misses()` | Cache stats: hits and misses counters |
| `crates/lcp-core/src/cache.rs:481-490` | `clear_entries_removes_all()` | Cache: `clear_entries()` deletes all |
| `crates/lcp-core/src/cache.rs:493-509` | `record_and_get_trace()` | Tracing: `record_trace()` and `get_trace()` |
| `crates/lcp-core/src/cache.rs:512-516` | `get_trace_unknown_returns_empty()` | Tracing: unknown trace ID returns empty |
| `crates/lcp-core/src/cache.rs:519-526` | `bytes_served_from_cache_incremented()` | Cache stats: `bytes_served_from_cache` |
| `crates/lcp-core/src/cache.rs:529-549` | `inspect_returns_full_entry()` | Cache: `inspect()` with no side effects |
| `crates/lcp-core/src/cache.rs:552-555` | `inspect_unknown_returns_none()` | Cache: `inspect()` on missing key |
| `crates/lcp-core/src/cache.rs:558-569` | `inspect_trace_returns_full_entries()` | Tracing: `inspect_trace()` |
| `crates/lcp-core/src/provider.rs:83-90` | `gemini_extract_model_from_generate_content()` | Model extraction: Gemini `generateContent` |
| `crates/lcp-core/src/provider.rs:93-99` | `gemini_extract_model_from_stream_generate_content()` | Model extraction: Gemini `streamGenerateContent` |
| `crates/lcp-core/src/provider.rs:102-108` | `gemini_extract_model_with_dashes_and_version()` | Model extraction: Gemini with dashes |
| `crates/lcp-core/src/provider.rs:111-116` | `gemini_extract_model_empty_returns_none()` | Model extraction: Gemini empty model |
| `crates/lcp-core/src/provider.rs:119-124` | `gemini_extract_model_no_marker_returns_none()` | Model extraction: Gemini missing marker |
| `crates/lcp-core/src/provider.rs:127-132` | `gemini_extract_model_no_colon_returns_none()` | Model extraction: Gemini missing colon |
| `crates/lcp-core/src/provider.rs:135-140` | `non_gemini_providers_return_none_from_path()` | Model extraction: non-Gemini returns None |

### Spec Invariant Tests (external, `tests/spec/`)

| File | Test | SPEC Coverage |
|---|---|---|
| `tests/spec/cache_miss.rs:26-45` | `test_miss_adds_x_lcp_cache_miss_header()` | Cache miss: `x-lcp-cache: MISS` header |
| `tests/spec/cache_miss.rs:48-65` | `test_miss_adds_x_lcp_key_header()` | Cache miss: `x-lcp-key` header (12 hex chars) |
| `tests/spec/cache_miss.rs:68-87` | `test_miss_stores_2xx_response()` | Cache miss: 2xx responses stored |
| `tests/spec/cache_miss.rs:90-113` | `test_miss_does_not_store_non_2xx()` | Cache miss: non-2xx NOT stored |
| `tests/spec/cache_miss.rs:116-136` | `test_miss_increments_misses_stat()` | Cache miss: misses counter incremented |
| `tests/spec/cache_hit.rs:41-60` | `test_hit_adds_x_lcp_cache_hit_header()` | Cache hit: `x-lcp-cache: HIT` header |
| `tests/spec/cache_hit.rs:63-80` | `test_hit_adds_x_lcp_key_header()` | Cache hit: `x-lcp-key` header |
| `tests/spec/cache_hit.rs:83-108` | `test_hit_increments_hit_count()` | Cache hit: per-entry `hit_count` incremented |
| `tests/spec/cache_hit.rs:111-137` | `test_hit_increments_stats()` | Cache hit: hits and bytes_served_from_cache incremented |
| `tests/spec/cache_hit.rs:140-165` | `test_hit_replays_stored_chunks()` | Cache hit: chunks replayed exactly, no upstream call |
| `tests/spec/model_extraction.rs:28-56` | `test_gemini_model_extracted_from_path()` | Model extraction: Gemini from URL path |
| `tests/spec/model_extraction.rs:59-84` | `test_gemini_model_appears_in_by_model_stats()` | Stats: Gemini model in `by_model` |
| `tests/spec/model_extraction.rs:87-117` | `test_gemini_stream_generate_content_model_extracted()` | Model extraction: Gemini `streamGenerateContent` |
| `tests/spec/model_extraction.rs:131-162` | `test_anthropic_model_extracted_from_body()` | Model extraction: Anthropic from JSON body |
| `tests/spec/model_extraction.rs:165-202` | `test_openai_model_extracted_from_body()` | Model extraction: OpenAI from JSON body |

### Test Infrastructure

| File | Purpose |
|---|---|
| `tests/common/mock_upstream.rs:33-290` | MockUpstream: HTTP mock server (JSON, SSE, error responses) |
| `tests/common/harness.rs:14-164` | TestHarness: wires MockUpstream + proxy + in-memory cache |

---

## 2. Coverage Map Against SPEC

### Providers (SPEC.md lines 31–51)

| Requirement | Status | Coverage |
|---|---|---|
| Four providers: Anthropic, OpenAI, OpenRouter, Gemini | ✅ Covered | `crates/lcp-core/src/provider.rs:5–10` enum definition |
| `from_prefix(s) -> Option<Provider>` | ✅ Covered | Unit test in `provider.rs:135–140` covers all four |
| `path_prefix()` returns canonical prefix | ✅ Covered | Implicit in all spec tests (routes work correctly) |
| `default_upstream()` returns correct URL | ✅ Covered | Unit test — not in spec tests but accessible via upstream_for() |
| Unknown prefix returns HTTP 404 | ❌ **Not covered** | No HTTP-level test for `GET /unknown/path` |

---

### Cache Key (SPEC.md lines 53–77)

| Requirement | Status | Coverage |
|---|---|---|
| Normalization rule 1: Parse JSON, fallback to raw | ✅ Covered | `hash.rs:125–130` test `invalid_json_hashes_raw()` |
| Normalization rule 2: Strip `stream` field | ✅ Covered | `hash.rs:86–93` test `stream_field_stripped()` |
| Normalization rule 3: Sort object keys recursively | ✅ Covered | `hash.rs:96–103` test `key_order_independent()` |
| Normalization rule 4: Compact JSON serialization | ✅ Covered | Implicit in all unit tests |
| BLAKE3 hex digest | ✅ Covered | `hash.rs:77–83` test `identical_bodies_same_key()` |
| Model field NOT stripped | ✅ Covered | Implicit (cache entries tracked by model in all tests) |
| Headers NOT included in key | ✅ Covered | Implicit (different auth tokens don't bust cache) |

---

### Cache Storage (SPEC.md lines 78–124)

| Requirement | Status | Coverage |
|---|---|---|
| SQLite storage at `$XDG_CACHE_HOME/lcp/cache.db` (or `~/.cache/lcp/cache.db`) | ⚠️ Partially covered | Tests use `:memory:` SQLite; path logic not tested |
| Overridable by `--db` or `LCP_DB` | ❌ **Not covered** | No CLI/config test |
| Only 2xx responses cached | ✅ Covered | `cache_miss.rs:90–113` test `test_miss_does_not_store_non_2xx()` |
| Non-2xx forwarded as-is, not stored | ✅ Covered | Same test as above |
| TTL configurable via `--ttl <seconds>` | ❌ **Not covered** | No TTL test (e.g., store + wait + verify miss) |
| TTL=0 means never expire | ❌ **Not covered** | Same as above |
| Expired entries treated as misses on read | ❌ **Not covered** | No time-based expiry test |
| Lazy expiry (not proactive deletion) | ❌ **Not covered** | Same as above |
| Schema: `entries` table with 10 fields | ✅ Covered | `cache.rs` unit tests verify all fields |
| Schema: `trace_entries` many-to-many | ✅ Covered | `cache.rs:493–509` test `record_and_get_trace()` |
| Schema: `stats` counters | ✅ Covered | `cache.rs:467–478` test `stats_track_hits_and_misses()` |

---

### Proxy Behavior: Cache miss (SPEC.md lines 128–142)

| MUST Requirement | Status | Coverage |
|---|---|---|
| 1. Forward request preserving method, path, query; strip host, connection, transfer-encoding, accept-encoding, content-length | ❌ **Not covered** | No test verifies which headers are stripped |
| 2. Stream each response chunk as it arrives; don't buffer | ✅ Covered | Implicit in `cache_miss.rs:26–45` (SSE chunks streamed) |
| 3. On 2xx, store exchange with chunks and timestamps | ✅ Covered | `cache_miss.rs:68–87` test `test_miss_stores_2xx_response()` |
| 4. Response includes `x-lcp-cache: MISS` and `x-lcp-key: <12-hex>` | ✅ Covered | `cache_miss.rs:26–45` and `cache_miss.rs:48–65` |
| 5. Increment `misses` stat | ✅ Covered | `cache_miss.rs:116–136` test `test_miss_increments_misses_stat()` |

---

### Proxy Behavior: Cache hit (SPEC.md lines 143–152)

| MUST Requirement | Status | Coverage |
|---|---|---|
| 1. Replay stored chunks sequentially at full speed, no delay | ✅ Covered | `cache_hit.rs:140–165` test `test_hit_replays_stored_chunks()` |
| 2. Preserve original chunk boundaries | ✅ Covered | Same test as above |
| 3. Response includes `x-lcp-cache: HIT` and `x-lcp-key: <12-hex>` | ✅ Covered | `cache_hit.rs:41–60` and `cache_hit.rs:63–80` |
| 4. Increment `hit_count` for the entry | ✅ Covered | `cache_hit.rs:83–108` test `test_hit_increments_hit_count()` |
| 5. Increment `hits` and `bytes_served_from_cache` stats | ✅ Covered | `cache_hit.rs:111–137` test `test_hit_increments_stats()` |

---

### Proxy Behavior: Bypass (SPEC.md lines 154–159)

| MUST Requirement | Status | Coverage |
|---|---|---|
| Request with `x-lcp-bypass: 1` skips cache read and write | ❌ **Not covered** | No test for bypass header |
| Response includes `x-lcp-cache: BYPASS` | ❌ **Not covered** | Same |
| Response MUST NOT include `x-lcp-key` header | ❌ **Not covered** | Same |
| Bypass request NOT recorded in `trace_entries` | ❌ **Not covered** | Same |

---

### Consumer Compression (SPEC.md lines 161–171)

| Requirement | Status | Coverage |
|---|---|---|
| MUST strip `Accept-Encoding` before forwarding | ❌ **Not covered** | No test sends Accept-Encoding and verifies it's stripped |
| MUST decompress compressed request body before hashing | ❌ **Not covered** | No test sends gzip/brotli request body |
| MUST return uncompressed responses; no re-encoding | ❌ **Not covered** | No test verifies decompression of request or no re-compression of response |

---

### SSE / Streaming (SPEC.md lines 173–185)

| Requirement | Status | Coverage |
|---|---|---|
| Cache miss: chunks forwarded as they arrive | ✅ Covered | Implicit in `cache_miss.rs` tests (SSE responses used) |
| Cache hit: stored chunks replayed sequentially at full speed | ✅ Covered | `cache_hit.rs:140–165` test `test_hit_replays_stored_chunks()` |
| Accept-Encoding stripped from all forwarded requests | ❌ **Not covered** | No explicit test; harness disables compression but client-side only |

---

### Tracing (SPEC.md lines 187–224)

| Requirement | Status | Coverage |
|---|---|---|
| Persist `(trace_id, cache_key)` for each non-bypass, traced request | ❌ **Not covered** | No HTTP-level test for `x-lcp-trace: <id>` header |
| Multiple requests in session share same trace ID | ❌ **Not covered** | No test sends multiple requests with same trace ID |
| Single cache entry may appear in multiple traces (many-to-many) | ⚠️ Partially covered | `cache.rs:493–509` tests basic record/get but not multi-trace scenario |
| Bypass requests NOT recorded in trace_entries | ❌ **Not covered** | No bypass test |
| `GET /trace/<trace-id>` returns metadata ordered by `created_at` | ❌ **Not covered** | No HTTP endpoint test |
| `GET /trace/<trace-id>?full=true` includes full request/response | ❌ **Not covered** | Same |
| Unknown trace ID returns empty `entries` array | ⚠️ Partially covered | `cache.rs:512–516` tests `get_trace()` on unknown ID, but no HTTP endpoint test |

---

### Stats and Admin Endpoints (SPEC.md lines 226–279)

| Endpoint | Status | Coverage |
|---|---|---|
| `GET /` health check | ❌ **Not covered** | No HTTP test (endpoint exists in `stats.rs:9–15`) |
| `GET /stats` aggregate stats | ❌ **Not covered** | No HTTP test (endpoint exists in `stats.rs:17–22`) |
| `DELETE /stats` reset counters | ❌ **Not covered** | No HTTP test (endpoint exists in `stats.rs:24–29`) |
| `DELETE /cache` purge entries | ❌ **Not covered** | No HTTP test (endpoint exists in `stats.rs:31–36`) |
| `GET /cache/<key>` full exchange | ❌ **Not covered** | No HTTP test (endpoint exists in `stats.rs:38–47`) |
| `GET /trace/<trace-id>` metadata | ❌ **Not covered** | No HTTP test (endpoint exists in `stats.rs:55–90`) |
| `GET /trace/<trace-id>?full=true` full data | ❌ **Not covered** | Same |

---

### Model Extraction (SPEC.md lines 88–108, lcp-server/SPEC.md lines 88–108)

| Provider | Extraction Rule | Status | Coverage |
|---|---|---|---|
| Anthropic | `model` field from JSON body | ✅ Covered | `model_extraction.rs:131–162` test `test_anthropic_model_extracted_from_body()` |
| OpenAI | `model` field from JSON body | ✅ Covered | `model_extraction.rs:165–202` test `test_openai_model_extracted_from_body()` |
| OpenRouter | `model` field from JSON body | ❌ **Not covered** | No specific test (should be identical to Anthropic/OpenAI) |
| Gemini | `/models/{model}:{verb}` from URL path | ✅ Covered | `model_extraction.rs:28–117` — multiple Gemini tests |
| Fallback: on failure, store `None` as model | ❌ **Not covered** | No test for malformed JSON or unrecognized path pattern |

---

### Configuration (SPEC.md lines 280–295)

| Requirement | Status | Coverage |
|---|---|---|
| `--port` / `LCP_PORT` (default 9001) | ❌ **Not covered** | No CLI test |
| `--host` / `LCP_HOST` (default 127.0.0.1) | ❌ **Not covered** | No CLI test |
| `--db` / `LCP_DB` (default `~/.cache/lcp/cache.db`) | ❌ **Not covered** | Tests use `:memory:` |
| `--ttl` / `LCP_TTL` (default 0) | ❌ **Not covered** | No TTL test |
| `--timeout` / `LCP_TIMEOUT` (default 300) | ❌ **Not covered** | No timeout test |
| `--anthropic-upstream` / `LCP_ANTHROPIC_UPSTREAM` | ❌ **Not covered** | Tests hardcode via harness |
| `--openai-upstream` / `LCP_OPENAI_UPSTREAM` | ❌ **Not covered** | Same |
| `--openrouter-upstream` / `LCP_OPENROUTER_UPSTREAM` | ❌ **Not covered** | Same |
| `--gemini-upstream` / `LCP_GEMINI_UPSTREAM` | ❌ **Not covered** | Same |
| CLI flags take priority over env vars | ❌ **Not covered** | No test for priority |

---

### Error Paths (SPEC.md + lcp-server/SPEC.md)

| Error Condition | Status | Coverage |
|---|---|---|
| Non-2xx responses not cached | ✅ Covered | `cache_miss.rs:90–113` test `test_miss_does_not_store_non_2xx()` |
| Non-2xx forwarded as-is | ✅ Covered | Same test — response is 400, not cached |
| Upstream timeout (default 300s) | ❌ **Not covered** | No test with slow/hanging upstream |
| Upstream unreachable | ❌ **Not covered** | No test for upstream connection failure |
| Unknown provider prefix returns 404 | ❌ **Not covered** | No HTTP test for `GET /badprovider/v1/path` |

---

## 3. Prioritized Gap List

### P1: Behavioral Correctness Gaps
**Why**: Spec requirements with no test carry silent-failure risk. Behavior may be missing entirely or partially implemented.

1. **Unknown provider prefix returns 404** (SPEC.md:51, lcp-server/SPEC.md:53)
   - **Why it matters**: Clients sending requests to invalid provider prefixes should get a clear error, not a silent 500 or pass-through.
   - **Test**: `GET /badprovider/v1/messages` → expects 404 with descriptive message
   - **File path**: `tests/spec/routing.rs` (new)

2. **Bypass header skips cache read/write and excludes x-lcp-key** (SPEC.md:156–159)
   - **Why it matters**: If bypass is not implemented correctly, clients may cache unintended requests or leak cache state via x-lcp-key on bypass.
   - **Test**: (a) `GET /anthropic/v1/messages` with `x-lcp-bypass: 1` → no cache write, (b) response includes `x-lcp-cache: BYPASS`, (c) x-lcp-key header absent, (d) verify upstream called twice for identical request (no cache hit possible).
   - **File path**: `tests/spec/bypass.rs` (new)

3. **Bypass request NOT recorded in trace_entries** (SPEC.md:204)
   - **Why it matters**: If bypass requests are traced, cache size may grow unbounded with bypass-only sessions.
   - **Test**: Request with both `x-lcp-trace: <id>` and `x-lcp-bypass: 1` → verify `trace_id` not found in `trace_entries` after request.
   - **File path**: Same as above (`bypass.rs`)

4. **Header stripping on upstream forwarding** (SPEC.md:131–132, lcp-server/SPEC.md:78–79)
   - **Why it matters**: Failing to strip `accept-encoding` causes upstream to compress SSE responses, breaking the proxy's chunking semantics. Failing to strip `host`, `content-length` may cause upstream to reject the request.
   - **Test**: (a) Request with `Host: custom.example.com` → verify upstream receives `Host: <upstream-domain>`, (b) Request with `Accept-Encoding: gzip` → verify upstream receives no Accept-Encoding, (c) Request with `Content-Length: 10` → verify reqwest replaces it correctly.
   - **File path**: `tests/spec/forwarding.rs` (new)

5. **Tracing endpoint (GET /trace/<trace-id>) returns metadata and full data** (SPEC.md:210–224)
   - **Why it matters**: Clients depend on the trace endpoint to correlate requests and responses during debugging. If the endpoint is missing or returns wrong data, debugging is impossible.
   - **Test**: (a) POST request with `x-lcp-trace: my-trace`, (b) `GET /trace/my-trace` → returns metadata for the cache key, (c) `GET /trace/my-trace?full=true` → returns full request+response, (d) Unknown trace ID → returns 200 with empty entries array.
   - **File path**: `tests/spec/tracing.rs` (new)

6. **Model extraction fallback (None on failure)** (lcp-server/SPEC.md:106–108)
   - **Why it matters**: Malformed requests should still be cached but with `model = None`. If the proxy crashes or skips caching on malformed bodies, cache miss rates spike.
   - **Test**: (a) OpenAI request with malformed JSON body → verify cache entry created with `model: null`, (b) Gemini request with path not matching `/models/{model}:` pattern → verify cached with `model: null`.
   - **File path**: `tests/spec/model_extraction.rs` (extend existing)

### P2: Error Path Gaps
**Why**: Missing error tests mean error conditions are untested. The proxy may crash, return wrong status, or leak internal errors.

7. **Upstream timeout** (lcp-server/SPEC.md:34–35)
   - **Why it matters**: Default 300s timeout protects against hanging upstreams. If unimplemented, slow upstreams hang clients indefinitely.
   - **Test**: Mock upstream that never responds, with `--timeout 1` → request should fail with 504 or 502 after 1s, not hang.
   - **File path**: `tests/integration/timeout.rs` (new)

8. **Upstream unreachable** (implied in error handling)
   - **Why it matters**: If the upstream is down, the proxy should return 502/503, not crash or return 500.
   - **Test**: Mock upstream not listening, proxy attempts to connect → expects 502 Bad Gateway.
   - **File path**: Same as above

9. **TTL expiry on cache read** (SPEC.md:87–88)
   - **Why it matters**: If TTL is not enforced, stale entries are served indefinitely. If TTL is always enforced (even when TTL=0), cache becomes ineffective.
   - **Test**: (a) Store entry with `--ttl 1`, wait 2s, read → expects miss and misses counter incremented, (b) Store entry with TTL=0, wait 10s, read → expects hit.
   - **File path**: `tests/integration/ttl.rs` (new)

---

### P3: Edge Case Gaps
**Why**: Specified but unlikely scenarios. May not block releases but indicate incomplete coverage.

10. **Multiple requests in same trace** (SPEC.md:202)
    - **Why it matters**: If trace doesn't aggregate multiple cache keys, trace output is incomplete.
    - **Test**: POST request 1 with `x-lcp-trace: session-1`, POST request 2 (different body) with same `x-lcp-trace: session-1`, `GET /trace/session-1` → returns 2 entries.
    - **File path**: `tests/spec/tracing.rs` (new)

11. **Single cache entry in multiple traces** (SPEC.md:203)
    - **Why it matters**: If a single cache key can't appear in multiple traces, trace isolation is broken.
    - **Test**: Request A with `trace-1`, request B (identical) with `trace-2`, both hit same cache key, verify both `trace-1` and `trace-2` reference the key.
    - **File path**: Same as above

12. **Consumer compression: Accept-Encoding stripping** (SPEC.md:163–171)
    - **Why it matters**: If Accept-Encoding is not stripped, upstream may send compressed SSE, which the proxy can't decompress mid-stream. This breaks streaming semantics.
    - **Test**: Request with `Accept-Encoding: gzip` → verify request forwarded to upstream without Accept-Encoding header, verify upstream response is uncompressed.
    - **File path**: `tests/spec/compression.rs` (new)

13. **Consumer compression: request body decompression** (SPEC.md:168–169)
    - **Why it matters**: If incoming compressed request bodies are not decompressed before hashing, two identical requests (one compressed, one not) produce different cache keys.
    - **Test**: POST with `Content-Encoding: gzip` and gzip-compressed body → verify hashed correctly and cache hit on subsequent uncompressed request.
    - **File path**: Same as above

14. **OpenRouter model extraction** (lcp-server/SPEC.md:97)
    - **Why it matters**: Spec says OpenRouter extracts model from body like Anthropic/OpenAI. No test verifies this specific provider.
    - **Test**: OpenRouter request with `model: "openrouter/anthropic/claude-sonnet-4"` → verify entry stored with model correctly.
    - **File path**: `tests/spec/model_extraction.rs` (extend)

15. **Configuration via CLI and env vars** (SPEC.md:280–295)
    - **Why it matters**: Users rely on env vars and CLI flags. If flags or env vars don't work, deployment is broken.
    - **Test**: (a) `LCP_PORT=9002 lcp` → listens on 9002, (b) `LCP_TTL=60 lcp` → entries expire after 60s, (c) `lcp --db /tmp/custom.db` → uses custom path, (d) `lcp --port 9003 --ttl 30` (CLI) vs `LCP_PORT=9002 LCP_TTL=60 lcp --port 9003 --ttl 30` → CLI takes precedence.
    - **File path**: `tests/integration/config.rs` (new) — **Note**: These are E2E and require actual CLI invocation. May be deferred to `--features test-e2e`.

---

### P4: Infrastructure/Configuration Gaps
**Why**: Low risk but indicates incomplete test suite maturity. Mostly non-functional.

16. **Admin endpoints (GET /, GET /stats, DELETE /stats, DELETE /cache, GET /cache/<key>)** (SPEC.md:230–234, lcp-server/SPEC.md)
    - **Why it matters**: Operations teams depend on `/stats` for monitoring and `/cache` for debugging.
    - **Test**: (a) `GET /` → returns `{"status":"ok"}`, (b) `GET /stats` → returns hits, misses, bytes_served_from_cache, entries, by_model, (c) after cache write, `DELETE /stats` → stats reset to 0 but entry persists, (d) `DELETE /cache` → all entries cleared but stats unchanged, (e) `GET /cache/<key>` → returns full entry or 404.
    - **File path**: `tests/spec/admin.rs` (new)

17. **by_model aggregation in stats** (SPEC.md:271–278)
    - **Why it matters**: Monitoring systems key off `by_model` for SLO tracking per model.
    - **Test**: Store entries for `claude-opus-4`, `gpt-4o`, `gemini-2.5-flash` (one each), `GET /stats` → `by_model` includes correct counts for each.
    - **File path**: Same as above (`admin.rs`)

---

## Summary Statistics

- **Total test cases found**: 42 (23 unit, 15 spec, 4 infrastructure)
- **Covered requirements**: 42 / 117 (36%)
- **P1 gaps**: 6 tests needed
- **P2 gaps**: 3 tests needed
- **P3 gaps**: 6 tests needed
- **P4 gaps**: 2 tests needed
- **Total new tests needed**: **17 test files/functions** to reach ~90% coverage

---

## Entry Point for Next Phase

**Start with P1 gaps** in this order:

1. **Unknown provider 404** — fastest, highest confidence in fix
2. **Bypass behavior** — touches core proxy logic; high confidence
3. **Header stripping** — affects upstream compatibility; verify against actual APIs in manual test
4. **Tracing endpoints** — adds new test surface but logic is already implemented
5. **Model extraction fallback** — extends existing model tests, low risk

Create `tests/spec/routing.rs`, `tests/spec/bypass.rs`, `tests/spec/forwarding.rs`, `tests/spec/tracing.rs` in parallel, then move to P2.
