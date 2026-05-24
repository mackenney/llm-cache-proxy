# Master Progress

Read this file to understand the current state of lcp. One-liner per item; all detail is in plan files and git history.

---

## In Progress

| Item | Plan | Notes |
|---|---|---|
| _(none)_ | | |

---

## Queued

| Plan | What |
|---|---|
| [test1](plans/test1/PROGRESS.md) | MockUpstream + spec invariant test suite (cache hit/miss, bypass, tracing, admin endpoints) |

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

---

## Known Gaps

| ID | Issue | Plan |
|---|---|---|
| TEST-1 | No `MockUpstream` — blocks spec invariant and integration tests for proxy behavior | [test1](plans/test1/PROGRESS.md) |
