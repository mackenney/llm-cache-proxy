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
| [tests1](plans/tests1/PROGRESS.md) | 20+ spec invariant + integration tests: routing, bypass, forwarding, tracing, admin, model extraction, compression, timeout, TTL (12 steps, 3 waves) |

---

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

---

## Known Gaps

| ID | Issue | Plan |
|---|---|---|
| _(none)_ | |
