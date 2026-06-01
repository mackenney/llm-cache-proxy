# Master Progress

Read this file to understand the current state of lcp. One-liner per item; all detail is in plan files and git history.

---

## In Progress

| Item | Plan | Notes |
|---|---|---|
| _(none)_ | |
---

## Queued

| Plan | What |
|---|---|
| _(future)_ | **System-prompt content normalization** — pattern-driven stripping of volatile lines (e.g. `Current date: …`, `Current working directory: …`) from system prompt content before cache key derivation. Patterns configured in `[extensions.system_prompt_scrub]` as a list of regexes applied to the system message body (Anthropic top-level `system` field or first `role:system` message). Key-only: forwarded payload is unchanged, so the model still receives real values. Motivation: agentic harnesses like pi inject the current date and CWD into every system prompt, busting the cache on every new day and every new project path. Pattern-driven design keeps lcp agnostic to any specific harness. |

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

---

## Known Gaps

| ID | Issue | Plan |
| SSE-unscrub | `unscrub_stream` uses raw-byte Aho-Corasick; fakes split across `content_block_delta` SSE events are never contiguous → Phase 3 silently passes the fake through. See `crates/lcp-server/SPEC.md §SSE-Aware Unscrubbing`. | none |
