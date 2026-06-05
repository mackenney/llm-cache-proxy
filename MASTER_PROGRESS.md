# Master Progress

Read this file to understand the current state of lcp. One-liner per item; all detail is in plan files and git history.

---

## In Progress

_(none)_

## Queued

### System-prompt cache-key normalization

Exclude volatile lines injected by agentic harnesses (e.g. `Current date: …`, `Current working directory: …`) from the cache key so the same logical prompt gets the same key across days and directories. The forwarded request is never modified — only the bytes fed into BLAKE3 are affected. Configured via `[extensions.system_prompt_normalize]` as a list of regexes matched against the system message body.

## Completed

### Foundation
- Cargo workspace, crate stubs, rustfmt/clippy config
- Root `SPEC.md`: full behavioral contract (proxy, cache key, providers, SSE, tracing, stats, config)
- `crates/lcp-core/SPEC.md` and `crates/lcp-server/SPEC.md`: per-crate behavioral contracts
- `AGENTS.md`, `MASTER_PROGRESS.md`: project constitution and governance
- `lcp-core` skeleton: types, provider enum, BLAKE3 hash, SQLite cache
- `lcp-server` skeleton: axum router, proxy handler, stats endpoints
- `lcp` CLI skeleton: clap binary

### Implementation
- Rename `its-classified` → `doppel`, `swap`/`restore` verbs, `SecretsFile`, `DoppelExt` throughout — 887d985
- Fix stale scrub/unscrub vocabulary in docs, tests, CLI strings, SPEC.md — 253ffa1
- Fix spec gaps: tracing, bypass headers, decompression, timeout, stats (17 tests) — 6dfbc71
- Gemini provider, `GET /cache/<key>`, `GET /trace/<id>?full=true`, `FullEntry`/`inspect`/`inspect_trace` — c0230a4
- `MockUpstream` + `TestHarness` + 10 Priority 1 spec invariant tests (cache hit/miss) — eda6d31
- Replace full-body buffering with true streaming: spawn+channel for miss, `stream::iter` for hit — 3ddc80e
- Fixups: rename `RecordedRequest.path→uri`, extract channel capacity, replace `yield_now` with deterministic drain — 83e2a19
- Gemini path-based model extraction: `extract_model_from_path` + proxy wiring + 5 spec invariant tests — fff8d01
- 32 new tests (routing, bypass, forwarding, tracing, admin, model extraction, compression, timeout, TTL); fix expired-entry miss counter gap — eab47cf
- Provider-aware cache key normalization: `normalization_strip_fields`, `cache_key(provider,…)`, 9 spec invariant tests — 4b01572
- Extension pipeline: Phase 1/2/3 hooks, `SensitiveState` opaque store, proxy wiring, 10 spec invariant tests — be99fd3
- Code review fixes: 3 BLOCKERs, 3 IMPORTANTs, 3 perf, 3 maintenance — f93fd81
- SSE-aware restoring: `SseRestoreStream` replaces raw-byte restore; all 4 providers covered — 6d30152
- Fix SSE detection for Anthropic real API: `is_sse_first_chunk` detects `event: ` prefix; E2E verified — 6e39cc0
- Code review fixes: frame reconstruction bug, UTF-8 chunk corruption, empty-chunk latch, partial-state error, hex crate, SPEC update (159 tests) — a834725

---

## Known Gaps

_(none)_
