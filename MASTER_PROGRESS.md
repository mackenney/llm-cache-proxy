# Master Progress

Read this file to understand the current state of lcp. One-liner per item; all detail is in plan files and git history.

---

## In Progress

| Item | Plan | Notes |
|---|---|---|
| **rename-to-doppel** | [plans/rename-to-doppel](./plans/rename-to-doppel/PROGRESS.md) | Rename `doppel` → `doppel`, `swap/restore` → `swap/restore`, all dependent types and files |
---

## Queued

| Plan | What |
|---|---|
| _(future)_ | **System-prompt cache-key normalization** — exclude volatile lines injected by agentic harnesses (e.g. `Current date: …`, `Current working directory: …`) from the cache key so the same logical prompt gets the same key across days and directories. **The forwarded request is never modified** — the provider always sees the real values. Only the bytes fed into BLAKE3 are affected. Configured via `[extensions.system_prompt_scrub]` as a list of regexes matched against the system message body (Anthropic top-level `system` field or first `role:system` message). Motivation: without this, every new day or working directory busts the entire cache even when the prompt logic is identical. |
## Completed

### Foundation

| What | Ref |
|---|---|
| Workspace skeleton: Cargo workspace, crate stubs, rustfmt/clippy config | _(scaffold)_ |
| Root SPEC.md: full behavioral contract (proxy, cache key, providers, SSE, tracing, stats, config) | _(spec)_ |
| AGENTS.md, MASTER_PROGRESS.md: project constitution and governance | _(governance)_ |
| lcp-core skeleton: types, provider enum, hash (BLAKE3, unit tests), cache (SQLite, unit tests) | _(scaffold)_ |
| lcp-server skeleton: axum router, proxy handler, stats endpoints | _(scaffold)_ |
| lcp CLI skeleton: clap binary | _(scaffold)_ |
| `crates/lcp-core/SPEC.md`: behavioral contract for Provider, cache key, Cache, types | _(spec)_ |
| `crates/lcp-server/SPEC.md`: behavioral contract for routing, proxy pipeline, SSE, tracing, admin endpoints | _(spec)_ |

### Implementation

| What | Ref |
|---|---|
| `rename-to-doppel` — rename `its-classified` → `doppel`, `swap`/`restore` verbs, `SecretsFile`, `DoppelExt` throughout | 887d985 |


| What | Ref |
|---|---|
| Fix spec gaps: tracing, bypass headers, decompression, timeout, stats (17 tests passing) | 6dfbc71 |
| Gemini provider, `GET /cache/<key>`, `GET /trace/<id>?full=true`, `FullEntry`/`inspect`/`inspect_trace` (17 tests passing) | c0230a4 |
| MockUpstream + TestHarness + 10 Priority 1 spec invariant tests (cache hit/miss, 41 tests passing) | eda6d31 |
| Replace full-body buffering with true streaming: spawn+channel for cache miss, stream::iter for cache hit | 3ddc80e |
| fixups1: rename RecordedRequest.path→uri, extract channel capacity, replace yield_now with deterministic drain | 83e2a19 |
| GEMINI-1: Gemini path-based model extraction — `extract_model_from_path` + proxy wiring + spec docs + 5 spec invariant tests | fff8d01 |
| tests1: 32 new tests (spec invariants + integration): routing, bypass, forwarding, tracing, admin, model extraction, compression, timeout, TTL; fix expired-entry miss counter gap | eab47cf |
| norm1: Provider-aware cache key normalization — `normalization_strip_fields`, `cache_key(provider,…)`, 9 spec invariant tests (85→94 tests) | 4b01572 |
| ext-1: Extension pipeline — Phase 1/2/3 hooks, SensitiveState opaque store, proxy wiring, 10 spec invariant tests (99→108 tests) | be99fd3 |
| cr1: code review fixes — 3 BLOCKERs, 3 IMPORTANTs, 3 perf, 3 maintenance (108→111 tests) | f93fd81 |
| SSE-aware restoring: `SseRestoreStream` replaces raw-byte restore for SSE responses; text accumulated across events, fake restored; all 4 providers covered | 6d30152 |
| Fix SSE detection for Anthropic real API: `is_sse_first_chunk` now detects `event: ` prefix; Phase 3 E2E verified live with fabricated-key roundtrip test | 6e39cc0 |
| cr2: review fixes — frame reconstruction bug, UTF-8 chunk corruption, empty-chunk latch, partial-state error, hex crate, SPEC update, 2 new tests (159 passing) | a834725 |

---

## Known Gaps

| ID | Issue | Plan |
| _(none)_ | | |
