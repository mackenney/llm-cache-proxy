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
| _(none yet)_ | Per-crate SPECs + implementation — next step is running planner against root SPEC.md |

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
| SPEC.md review: fixed schema (provider+model columns, stats table), bypass headers, timeout config, streaming note | _(spec-review)_ |
| `crates/lcp-core/SPEC.md`: behavioral contract for Provider, cache key, Cache, types | _(spec)_ |
| `crates/lcp-server/SPEC.md`: behavioral contract for routing, proxy pipeline, SSE, tracing, admin endpoints | _(spec)_ |

---

## Known Gaps

| ID | Issue | Plan |
|---|---|---|
| CORE-1 | `cargo check` compilation errors not yet verified — skeletons may not compile clean | _(no plan)_ |
| TEST-1 | `MockUpstream` not implemented — blocks all spec invariant and integration tests | _(no plan)_ |
| IMPL-1 | Skeleton deviations from spec: Gemini provider missing; proxy buffers full response instead of streaming; `x-lcp-key` absent on HIT responses; `bytes_served_from_cache` not incremented on hit; `trace_entries` table absent; `by_model` groups by model only not provider+model | _(no plan)_ |
