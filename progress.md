# Progress

## Status

In Progress

## Tasks

- [x] Repo scaffold: workspace, crates, .gitignore, rustfmt/clippy config
- [x] SPEC.md: full behavioral contract (proxy, cache key, providers, endpoints)
- [x] TESTING.md: test strategy and tier layout
- [x] AGENTS.md: agent guidelines
- [x] lcp-core skeleton: types, provider enum, hash (with unit tests), cache (with unit tests)
- [x] lcp-server skeleton: axum router, proxy handler, stats endpoints
- [x] lcp binary skeleton: CLI with clap
- [ ] Fix compilation errors and verify `cargo check` passes
- [ ] Write `tests/common/mock_upstream.rs`
- [ ] Spec invariant tests: routing, cache hit/miss flow, header forwarding, bypass
- [ ] Integration tests: full server, multi-provider, concurrent
- [ ] crates/lcp-core/SPEC.md
- [ ] crates/lcp-server/SPEC.md

## Notes

Reference implementation cloned at `references/llm-cache-proxy/` (git-ignored).
Key observations from the reference:
- Cache key: `sha256(method + "|" + path + "|" + body)` — no normalization, no stream stripping.
  lcp improves on this with blake3 + normalized JSON (stream stripped, keys sorted).
- Streaming: reference buffers full SSE body, replays verbatim. lcp does the same for v1.
- Single-file FastAPI server (~300 LOC). lcp is structured as a proper workspace for extensibility.
